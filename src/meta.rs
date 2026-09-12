use crate::{inv, Res};

pub const MAGIC: [u8; 4] = [0x49, 0x4D, 0x58, 0xA7];
pub const VERSION: u16 = 1;
pub const HEADER_LEN: usize = 64;
pub const CHECKSUM_OFF: usize = 32;
pub const CHECKSUM_LEN: usize = 32;
const ENTRY_FLAGS_LEN: usize = 2;
const ENTRY_FIXED: usize = 8 + 8 + 8 + ENTRY_FLAGS_LEN + 2;
const TAIL_LEN_LEN: usize = 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub offset: u64,
    pub size: u64,
    pub added: u64,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Meta {
    pub current: u32,
    pub last_changed: u64,
    pub last_added: u64,
    pub entries: Vec<Entry>,
    pub tail: Vec<u8>,
    pub checksum: [u8; CHECKSUM_LEN],
}

impl Meta {
    pub fn serialized_len(entries: &[Entry], tail_len: usize) -> usize {
        HEADER_LEN
            + entries
                .iter()
                .map(|e| ENTRY_FIXED + e.name.len())
                .sum::<usize>()
            + TAIL_LEN_LEN
            + tail_len
    }

    /// Serializes the meta with a zeroed checksum field. The caller patches
    /// `CHECKSUM_OFF..CHECKSUM_OFF+CHECKSUM_LEN` after hashing.
    pub fn serialize(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(Self::serialized_len(&self.entries, self.tail.len()));
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // flags, must be 0 in v1
        out.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.current.to_le_bytes());
        out.extend_from_slice(&self.last_changed.to_le_bytes());
        out.extend_from_slice(&self.last_added.to_le_bytes());
        out.extend_from_slice(&[0u8; CHECKSUM_LEN]);
        for e in &self.entries {
            out.extend_from_slice(&e.offset.to_le_bytes());
            out.extend_from_slice(&e.size.to_le_bytes());
            out.extend_from_slice(&e.added.to_le_bytes());
            out.extend_from_slice(&[0u8; ENTRY_FLAGS_LEN]);
            out.extend_from_slice(&(e.name.len() as u16).to_le_bytes());
            out.extend_from_slice(e.name.as_bytes());
        }
        out.extend_from_slice(&(self.tail.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.tail);
        out
    }

    /// Parses meta starting at `buf[0]`; returns the meta and its byte length.
    pub fn parse(buf: &[u8]) -> Res<(Meta, usize)> {
        if buf.len() < HEADER_LEN {
            return Err(inv("meta: truncated header"));
        }
        if buf[..4] != MAGIC {
            return Err(inv("meta: bad magic"));
        }
        let version = u16::from_le_bytes([buf[4], buf[5]]);
        if version != VERSION {
            return Err(inv(format!("meta: unsupported version {version}")));
        }
        let flags = u16::from_le_bytes([buf[6], buf[7]]);
        if flags != 0 {
            return Err(inv("meta: unknown header flags"));
        }
        let count = u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]) as usize;
        if count == 0 {
            return Err(inv("meta: zero images"));
        }
        if count > buf.len().saturating_sub(HEADER_LEN) / ENTRY_FIXED + 1 {
            return Err(inv("meta: entry count exceeds available data"));
        }
        let current = u32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]);
        let last_changed = u64::from_le_bytes(buf[16..24].try_into().unwrap());
        let last_added = u64::from_le_bytes(buf[24..32].try_into().unwrap());
        let mut checksum = [0u8; CHECKSUM_LEN];
        checksum.copy_from_slice(&buf[CHECKSUM_OFF..CHECKSUM_OFF + CHECKSUM_LEN]);

        let mut pos = HEADER_LEN;
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            if buf.len() < pos + ENTRY_FIXED {
                return Err(inv("meta: truncated entry"));
            }
            let offset = u64::from_le_bytes(buf[pos..pos + 8].try_into().unwrap());
            let size = u64::from_le_bytes(buf[pos + 8..pos + 16].try_into().unwrap());
            let added = u64::from_le_bytes(buf[pos + 16..pos + 24].try_into().unwrap());
            let eflags = u16::from_le_bytes([buf[pos + 24], buf[pos + 25]]);
            if eflags != 0 {
                return Err(inv("meta: unknown entry flags"));
            }
            let nlen = u16::from_le_bytes([buf[pos + 26], buf[pos + 27]]) as usize;
            pos += ENTRY_FIXED;
            if nlen == 0 {
                return Err(inv("meta: empty filename"));
            }
            if buf.len() < pos + nlen {
                return Err(inv("meta: truncated filename"));
            }
            let name = std::str::from_utf8(&buf[pos..pos + nlen])
                .map_err(|_| inv("meta: filename is not valid UTF-8"))?
                .to_string();
            if name.contains(['/', '\\', '\0']) {
                return Err(inv("meta: filename contains path separator or NUL"));
            }
            pos += nlen;
            entries.push(Entry {
                offset,
                size,
                added,
                name,
            });
        }
        if current as usize >= count {
            return Err(inv("meta: current index out of range"));
        }
        if buf.len() < pos + TAIL_LEN_LEN {
            return Err(inv("meta: truncated tail length"));
        }
        let tail_len = u32::from_le_bytes(buf[pos..pos + 4].try_into().unwrap()) as usize;
        pos += TAIL_LEN_LEN;
        if buf.len() < pos + tail_len {
            return Err(inv("meta: truncated tail"));
        }
        let tail = buf[pos..pos + tail_len].to_vec();
        pos += tail_len;
        Ok((
            Meta {
                current,
                last_changed,
                last_added,
                entries,
                tail,
                checksum,
            },
            pos,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Meta {
        Meta {
            current: 1,
            last_changed: 111,
            last_added: 222,
            entries: vec![
                Entry {
                    offset: 0,
                    size: 3,
                    added: 10,
                    name: "a.png".into(),
                },
                Entry {
                    offset: 99,
                    size: 4,
                    added: 20,
                    name: "b.jpg".into(),
                },
            ],
            tail: vec![1, 2, 3],
            checksum: [7u8; 32],
        }
    }

    #[test]
    fn roundtrip() {
        let m = sample();
        let bytes = m.serialize();
        assert_eq!(bytes.len(), Meta::serialized_len(&m.entries, m.tail.len()));
        let (parsed, len) = Meta::parse(&bytes).unwrap();
        assert_eq!(len, bytes.len());
        assert_eq!(parsed.entries, m.entries);
        assert_eq!(parsed.current, m.current);
        assert_eq!(parsed.tail, m.tail);
        assert_eq!(parsed.last_changed, 111);
        assert_eq!(parsed.last_added, 222);
        // checksum bytes are zero in serialized form until patched
        assert_eq!(&parsed.checksum, &[0u8; 32]);
    }

    #[test]
    fn rejects_bad_magic_and_versions() {
        let mut bytes = sample().serialize();
        bytes[0] ^= 1;
        assert!(Meta::parse(&bytes).is_err());

        let mut bytes = sample().serialize();
        bytes[4] = 9;
        assert!(Meta::parse(&bytes).is_err());

        let mut bytes = sample().serialize();
        bytes[12] = 9; // current out of range
        assert!(Meta::parse(&bytes).is_err());

        let mut bytes = sample().serialize();
        bytes[HEADER_LEN + 26] = 0;
        bytes[HEADER_LEN + 27] = 0; // zero-length filename
        assert!(Meta::parse(&bytes).is_err());
    }
}
