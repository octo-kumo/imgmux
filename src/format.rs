use crate::{inv, Res};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format {
    Png,
    Jpeg,
    Gif,
    WebP,
}

impl Format {
    pub fn name(self) -> &'static str {
        match self {
            Format::Png => "PNG",
            Format::Jpeg => "JPEG",
            Format::Gif => "GIF",
            Format::WebP => "WebP",
        }
    }
}

pub fn detect(buf: &[u8]) -> Option<Format> {
    if buf.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(Format::Png)
    } else if buf.starts_with(&[0xFF, 0xD8]) {
        Some(Format::Jpeg)
    } else if buf.starts_with(b"GIF87a") || buf.starts_with(b"GIF89a") {
        Some(Format::Gif)
    } else if buf.len() >= 12 && &buf[..4] == b"RIFF" && &buf[8..12] == b"WEBP" {
        Some(Format::WebP)
    } else {
        None
    }
}

/// Parse the image starting at `buf[0]` and return its format and exact end offset.
/// Only structure is walked; no pixels are decoded.
pub fn end_of_image(buf: &[u8]) -> Res<(Format, usize)> {
    match detect(buf) {
        Some(Format::Png) => png_end(buf).map(|e| (Format::Png, e)),
        Some(Format::Jpeg) => jpeg_end(buf).map(|e| (Format::Jpeg, e)),
        Some(Format::Gif) => gif_end(buf).map(|e| (Format::Gif, e)),
        Some(Format::WebP) => webp_end(buf).map(|e| (Format::WebP, e)),
        None => Err(inv(
            "unrecognized image format (expected PNG, JPEG, GIF, or WebP)",
        )),
    }
}

fn need(buf: &[u8], end: usize, what: &str) -> Res<()> {
    if end > buf.len() {
        Err(inv(format!("truncated {what}")))
    } else {
        Ok(())
    }
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

fn le32(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

fn be16(b: &[u8]) -> u16 {
    u16::from_be_bytes([b[0], b[1]])
}

fn png_end(buf: &[u8]) -> Res<usize> {
    let mut pos = 8;
    loop {
        need(buf, pos + 8, "PNG chunk header")?;
        let len = be32(&buf[pos..]) as usize;
        let ctype = &buf[pos + 4..pos + 8];
        let total = len
            .checked_add(12)
            .ok_or_else(|| inv("PNG chunk length overflow"))?;
        need(buf, pos + total, "PNG chunk")?;
        if ctype == b"IEND" {
            if len != 0 {
                return Err(inv("PNG IEND chunk is not empty"));
            }
            return Ok(pos + total);
        }
        pos += total;
    }
}

fn jpeg_end(buf: &[u8]) -> Res<usize> {
    let mut pos = 2;
    loop {
        need(buf, pos + 1, "JPEG marker")?;
        if buf[pos] != 0xFF {
            return Err(inv("JPEG: expected marker"));
        }
        while pos < buf.len() && buf[pos] == 0xFF {
            pos += 1;
        }
        need(buf, pos + 1, "JPEG marker")?;
        let marker = buf[pos];
        pos += 1;
        match marker {
            0xD8 | 0x01 | 0xD0..=0xD7 => continue,
            0xD9 => return Ok(pos),
            0xDA => {
                need(buf, pos + 2, "JPEG SOS segment")?;
                let seg = be16(&buf[pos..]) as usize;
                if seg < 2 {
                    return Err(inv("JPEG: bad SOS length"));
                }
                need(buf, pos + seg, "JPEG SOS segment")?;
                pos += seg;
                loop {
                    need(buf, pos + 1, "JPEG entropy data")?;
                    if buf[pos] != 0xFF {
                        pos += 1;
                        continue;
                    }
                    need(buf, pos + 2, "JPEG entropy data")?;
                    let b = buf[pos + 1];
                    if b == 0x00 || (0xD0..=0xD7).contains(&b) {
                        pos += 2;
                        continue;
                    }
                    break;
                }
            }
            0x00 => return Err(inv("JPEG: unexpected stuffed byte")),
            _ => {
                need(buf, pos + 2, "JPEG segment")?;
                let seg = be16(&buf[pos..]) as usize;
                if seg < 2 {
                    return Err(inv("JPEG: bad segment length"));
                }
                need(buf, pos + seg, "JPEG segment")?;
                pos += seg;
            }
        }
    }
}

fn gif_sub_blocks(buf: &[u8], mut pos: usize) -> Res<usize> {
    loop {
        need(buf, pos + 1, "GIF sub-block")?;
        let n = buf[pos] as usize;
        pos += 1;
        if n == 0 {
            return Ok(pos);
        }
        need(buf, pos + n, "GIF sub-block")?;
        pos += n;
    }
}

fn gif_end(buf: &[u8]) -> Res<usize> {
    need(buf, 13, "GIF header")?;
    let packed = buf[10];
    let mut pos = 13usize;
    if packed & 0x80 != 0 {
        let n = 3usize << ((packed & 0x07) + 1);
        need(buf, pos + n, "GIF color table")?;
        pos += n;
    }
    loop {
        need(buf, pos + 1, "GIF block")?;
        match buf[pos] {
            0x3B => return Ok(pos + 1),
            0x2C => {
                need(buf, pos + 10, "GIF image descriptor")?;
                let ipacked = buf[pos + 9];
                pos += 10;
                if ipacked & 0x80 != 0 {
                    let n = 3usize << ((ipacked & 0x07) + 1);
                    need(buf, pos + n, "GIF local color table")?;
                    pos += n;
                }
                need(buf, pos + 1, "GIF LZW code size")?;
                pos += 1;
                pos = gif_sub_blocks(buf, pos)?;
            }
            0x21 => {
                need(buf, pos + 2, "GIF extension")?;
                pos += 2;
                pos = gif_sub_blocks(buf, pos)?;
            }
            _ => return Err(inv("GIF: unknown block")),
        }
    }
}

fn webp_end(buf: &[u8]) -> Res<usize> {
    need(buf, 12, "WebP header")?;
    let riff = le32(&buf[4..]) as usize;
    let end = riff
        .checked_add(8)
        .ok_or_else(|| inv("WebP RIFF size overflow"))?;
    if end < 12 {
        return Err(inv("WebP: bad RIFF size"));
    }
    need(buf, end, "WebP data")?;
    let mut pos = 12;
    while pos < end {
        need(buf, pos + 8, "WebP chunk header")?;
        let csize = le32(&buf[pos + 4..]) as usize;
        pos = pos
            .checked_add(8)
            .and_then(|p| p.checked_add(csize))
            .ok_or_else(|| inv("WebP chunk overflow"))?;
        if pos > end {
            return Err(inv("WebP: chunk exceeds RIFF size"));
        }
        pos += csize & 1;
        if pos > end {
            return Err(inv("WebP: chunk padding exceeds RIFF size"));
        }
    }
    if pos != end {
        return Err(inv("WebP: trailing data inside RIFF"));
    }
    Ok(end)
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn png() -> Vec<u8> {
        let mut b = b"\x89PNG\r\n\x1a\n".to_vec();
        b.extend_from_slice(&13u32.to_be_bytes());
        b.extend_from_slice(b"IHDR");
        b.extend_from_slice(&[0, 0, 0, 1, 0, 0, 0, 1, 8, 0, 0, 0, 0]);
        b.extend_from_slice(&[0, 0, 0, 0]);
        b.extend_from_slice(&0u32.to_be_bytes());
        b.extend_from_slice(b"IEND");
        b.extend_from_slice(&[0xAE, 0x42, 0x60, 0x82]);
        b
    }

    pub fn jpeg() -> Vec<u8> {
        let mut b = vec![0xFF, 0xD8];
        b.extend_from_slice(&[0xFF, 0xE0]);
        let payload = b"JFIF\0\x01\x01\x00\x00\x01\x00\x01\x00\x00";
        b.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
        b.extend_from_slice(payload);
        b.extend_from_slice(&[0xFF, 0xC0]);
        b.extend_from_slice(&17u16.to_be_bytes());
        b.extend_from_slice(&[8, 0, 1, 0, 1, 3, 1, 0x11, 0, 2, 0x11, 0, 3, 0x11, 0]);
        b.extend_from_slice(&[0xFF, 0xDA]);
        b.extend_from_slice(&12u16.to_be_bytes());
        b.extend_from_slice(&[3, 1, 0, 2, 0, 3, 0, 0, 0x3F, 0]);
        b.extend_from_slice(&[0x12, 0xFF, 0x00, 0x34]);
        b.extend_from_slice(&[0xFF, 0xD9]);
        b
    }

    pub fn gif() -> Vec<u8> {
        let mut b = b"GIF89a".to_vec();
        b.extend_from_slice(&[1, 0, 1, 0, 0x00, 0, 0]);
        b.push(0x2C);
        b.extend_from_slice(&[0, 0, 0, 0, 1, 0, 1, 0, 0x00]);
        b.push(0x02);
        b.push(0x01);
        b.push(0x00);
        b.push(0x00);
        b.push(0x3B);
        b
    }

    pub fn webp() -> Vec<u8> {
        let mut b = b"RIFF".to_vec();
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(b"WEBP");
        b.extend_from_slice(b"VP8L");
        let data = [0x2F, 0, 0, 0, 0];
        b.extend_from_slice(&(data.len() as u32).to_le_bytes());
        b.extend_from_slice(&data);
        b.push(0);
        let total = b.len();
        b[4..8].copy_from_slice(&((total - 8) as u32).to_le_bytes());
        b
    }

    fn check(img: Vec<u8>, fmt: Format) {
        assert_eq!(end_of_image(&img).unwrap(), (fmt, img.len()));
        let mut trailing = img.clone();
        trailing.extend_from_slice(b"junk past the end");
        assert_eq!(end_of_image(&trailing).unwrap(), (fmt, img.len()));
        for i in 0..img.len() {
            assert!(
                end_of_image(&img[..i]).is_err(),
                "{fmt:?} truncation at {i} accepted"
            );
        }
    }

    #[test]
    fn ends() {
        check(png(), Format::Png);
        check(jpeg(), Format::Jpeg);
        check(gif(), Format::Gif);
        check(webp(), Format::WebP);
    }

    #[test]
    fn rejects_malformed() {
        let mut p = png();
        p[8..12].copy_from_slice(&0xFFFF_FFFFu32.to_be_bytes());
        assert!(end_of_image(&p).is_err());

        let mut j = jpeg();
        j.pop();
        assert!(end_of_image(&j).is_err());

        let mut g = gif();
        g[13] = 0x7F;
        assert!(end_of_image(&g).is_err());

        let mut w = webp();
        w[4..8].copy_from_slice(&999u32.to_le_bytes());
        assert!(end_of_image(&w).is_err());
    }
}
