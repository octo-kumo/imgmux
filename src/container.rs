use crate::format::{self, Format};
use crate::meta::{self, Meta};
use crate::{inv, Res};
use memmap2::Mmap;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

pub enum ImageData<'a> {
    Mapped(&'a [u8]),
    Owned(Vec<u8>),
}

impl ImageData<'_> {
    pub fn as_slice(&self) -> &[u8] {
        match self {
            ImageData::Mapped(s) => s,
            ImageData::Owned(v) => v,
        }
    }
}

pub struct Image<'a> {
    pub name: String,
    pub added: u64,
    pub data: ImageData<'a>,
}

pub struct Validated {
    pub meta: Meta,
    pub meta_start: usize,
    pub meta_len: usize,
    pub current_end: usize,
    pub current_format: Format,
}

pub struct Container {
    mmap: Mmap,
    pub path: PathBuf,
    pub meta: Meta,
    pub meta_start: usize,
    pub meta_len: usize,
    pub current_end: usize,
    pub current_format: Format,
}

impl Container {
    pub fn open(path: &Path) -> Res<Container> {
        let file = File::open(path).map_err(|e| inv(format!("{}: {e}", path.display())))?;
        let len = file.metadata()?.len();
        if len < (meta::HEADER_LEN + 16) as u64 {
            return Err(inv(format!(
                "{}: too small to be an imgmux",
                path.display()
            )));
        }
        // SAFETY: the file is only read through the map; mutations are done by
        // writing a fresh file and atomically renaming over this one.
        let mmap = unsafe { Mmap::map(&file)? };
        let v = validate(&mmap).map_err(|e| inv(format!("{}: {e}", path.display())))?;
        Ok(Container {
            mmap,
            path: path.to_path_buf(),
            meta: v.meta,
            meta_start: v.meta_start,
            meta_len: v.meta_len,
            current_end: v.current_end,
            current_format: v.current_format,
        })
    }

    pub fn data(&self) -> &[u8] {
        &self.mmap
    }

    pub fn count(&self) -> usize {
        self.meta.entries.len()
    }

    pub fn current(&self) -> usize {
        self.meta.current as usize
    }

    pub fn image(&self, i: usize) -> &[u8] {
        let e = &self.meta.entries[i];
        let start = e.offset as usize;
        &self.mmap[start..start + e.size as usize]
    }

    pub fn images(&self) -> Vec<Image<'_>> {
        self.meta
            .entries
            .iter()
            .enumerate()
            .map(|(i, e)| Image {
                name: e.name.clone(),
                added: e.added,
                data: ImageData::Mapped(self.image(i)),
            })
            .collect()
    }
}

/// Full strict validation of an in-memory container.
pub fn validate(data: &[u8]) -> Res<Validated> {
    let (cur_fmt, cur_end) = format::end_of_image(data)?;
    let meta_start = cur_end;
    if data.len() < meta_start + meta::HEADER_LEN {
        return Err(inv("meta: truncated"));
    }
    let (meta, meta_len) = Meta::parse(&data[meta_start..])?;
    let current = meta.current as usize;
    let ce = &meta.entries[current];
    if ce.offset != 0 || ce.size as usize != cur_end {
        return Err(inv("current image does not match its meta entry"));
    }

    let meta_end = meta_start
        .checked_add(meta_len)
        .ok_or_else(|| inv("meta: size overflow"))?;
    if meta_end > data.len() {
        return Err(inv("meta: extends past end of file"));
    }

    let mut expect = meta_end;
    for (i, e) in meta.entries.iter().enumerate() {
        if i == current {
            continue;
        }
        if e.offset as usize != expect {
            return Err(inv("image offsets are not contiguous"));
        }
        let size = e.size as usize;
        if size == 0 {
            return Err(inv("zero-sized image entry"));
        }
        let end = expect
            .checked_add(size)
            .ok_or_else(|| inv("image size overflow"))?;
        if end > data.len() {
            return Err(inv("image extends past end of file"));
        }
        let (_, parsed_end) = format::end_of_image(&data[expect..end])?;
        if parsed_end != size {
            return Err(inv("image size does not match actual data"));
        }
        expect = end;
    }
    if expect != data.len() {
        return Err(inv("trailing data after last image"));
    }

    let cs_off = meta_start + meta::CHECKSUM_OFF;
    let mut hasher = blake3::Hasher::new();
    hasher.update(&data[..cs_off]);
    hasher.update(&[0u8; meta::CHECKSUM_LEN]);
    hasher.update(&data[cs_off + meta::CHECKSUM_LEN..]);
    if hasher.finalize().as_bytes() != &meta.checksum {
        return Err(inv("container checksum mismatch"));
    }

    Ok(Validated {
        meta,
        meta_start,
        meta_len,
        current_end: cur_end,
        current_format: cur_fmt,
    })
}

/// Build a complete, checksummed container in memory as `(meta, meta_bytes)`.
/// The images are laid out as `[current][meta][others in entry order]`.
pub fn assemble(
    imgs: &[Image<'_>],
    current: usize,
    last_changed: u64,
    last_added: u64,
    tail: &[u8],
) -> Res<(Meta, Vec<u8>)> {
    let n = imgs.len();
    if n == 0 || current >= n {
        return Err(inv("internal: invalid image set"));
    }
    let drafts: Vec<meta::Entry> = imgs
        .iter()
        .map(|i| meta::Entry {
            offset: 0,
            size: i.data.as_slice().len() as u64,
            added: i.added,
            name: i.name.clone(),
        })
        .collect();
    let meta_len = Meta::serialized_len(&drafts, tail.len());
    let mut cursor = imgs[current].data.as_slice().len() + meta_len;
    let mut entries = Vec::with_capacity(n);
    for (i, d) in drafts.into_iter().enumerate() {
        let offset = if i == current {
            0
        } else {
            let o = cursor as u64;
            cursor += d.size as usize;
            o
        };
        entries.push(meta::Entry { offset, ..d });
    }

    let mut meta = Meta {
        current: current as u32,
        last_changed,
        last_added,
        entries,
        tail: tail.to_vec(),
        checksum: [0u8; meta::CHECKSUM_LEN],
    };
    let mut meta_bytes = meta.serialize();
    if meta_bytes.len() != meta_len {
        return Err(inv("internal: meta length mismatch"));
    }

    let mut hasher = blake3::Hasher::new();
    hasher.update(imgs[current].data.as_slice());
    hasher.update(&meta_bytes); // checksum field is zero here
    for (i, img) in imgs.iter().enumerate() {
        if i != current {
            hasher.update(img.data.as_slice());
        }
    }
    let hash = hasher.finalize();
    meta_bytes[meta::CHECKSUM_OFF..meta::CHECKSUM_OFF + meta::CHECKSUM_LEN]
        .copy_from_slice(hash.as_bytes());
    meta.checksum.copy_from_slice(hash.as_bytes());
    Ok((meta, meta_bytes))
}

pub fn write_container(
    w: &mut impl Write,
    imgs: &[Image<'_>],
    current: usize,
    meta_bytes: &[u8],
) -> std::io::Result<()> {
    w.write_all(imgs[current].data.as_slice())?;
    w.write_all(meta_bytes)?;
    for (i, img) in imgs.iter().enumerate() {
        if i != current {
            w.write_all(img.data.as_slice())?;
        }
    }
    Ok(())
}
