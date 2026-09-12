use crate::{inv, Res};
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Write via a same-directory temp file, fsync, atomic rename, fsync parent dir.
pub fn atomic_write(path: &Path, write: impl FnOnce(&mut File) -> Res<()>) -> Res<()> {
    let dir: PathBuf = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let base = path
        .file_name()
        .ok_or_else(|| inv(format!("invalid output path: {}", path.display())))?
        .to_string_lossy()
        .into_owned();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let tmp = dir.join(format!(".{base}.imgmux.{}.{nanos}.tmp", std::process::id()));

    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .map_err(|e| inv(format!("{}: {e}", tmp.display())))?;

    let result = (|| -> Res<()> {
        write(&mut f)?;
        f.sync_all()?;
        Ok(())
    })();
    if let Err(e) = result {
        drop(f);
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    drop(f);

    if let Ok(md) = fs::metadata(path) {
        let _ = fs::set_permissions(&tmp, md.permissions());
    }
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(inv(format!("rename to {}: {e}", path.display())));
    }
    if let Ok(d) = File::open(&dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

/// Unix seconds -> `YYYY-MM-DD HH:MM:SSZ` (UTC, proleptic Gregorian).
pub fn fmt_unix(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (hh, mi, ss) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    format!("{y:04}-{mo:02}-{d:02} {hh:02}:{mi:02}:{ss:02}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_format() {
        assert_eq!(fmt_unix(0), "1970-01-01 00:00:00Z");
        assert_eq!(fmt_unix(1_000_000_000), "2001-09-09 01:46:40Z");
        assert_eq!(fmt_unix(1_755_000_000), "2025-08-12 12:00:00Z");
    }
}
