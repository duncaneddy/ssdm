//! Persistence on the mounted volume: status.json (the change-detection source of
//! truth) and a mirror of every product's latest bytes under its R2 key path.

use std::path::Path;

use anyhow::{Context, Result};

use crate::status::{parse_status, serialize_status, Status};

const STATUS_FILE: &str = "status.json";

/// Load status.json from the volume; absent or corrupt yields an empty map.
pub fn load_status(data_dir: &Path) -> Status {
    match std::fs::read(data_dir.join(STATUS_FILE)) {
        Ok(bytes) => parse_status(&bytes),
        Err(_) => Status::new(),
    }
}

/// Atomically write status.json: write a temp file in the same dir, then rename.
pub fn save_status(data_dir: &Path, status: &Status) -> Result<()> {
    std::fs::create_dir_all(data_dir).with_context(|| format!("create {data_dir:?}"))?;
    let tmp = data_dir.join(format!("{STATUS_FILE}.tmp"));
    let final_path = data_dir.join(STATUS_FILE);
    std::fs::write(&tmp, serialize_status(status)).with_context(|| format!("write {tmp:?}"))?;
    std::fs::rename(&tmp, &final_path).with_context(|| format!("rename to {final_path:?}"))?;
    Ok(())
}

/// Write a product's bytes to `<data_dir>/<key>`, creating parent directories.
pub fn write_mirror(data_dir: &Path, key: &str, bytes: &[u8]) -> Result<()> {
    let path = data_dir.join(key);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {parent:?}"))?;
    }
    std::fs::write(&path, bytes).with_context(|| format!("write {path:?}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status::apply_update;

    #[test]
    fn status_round_trips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_status(dir.path()).is_empty(), "absent => empty");
        let mut s = Status::new();
        apply_update(&mut s, "eop/iers/x/latest/f.txt", "h", 3, 1000);
        save_status(dir.path(), &s).unwrap();
        assert_eq!(load_status(dir.path()), s);
    }

    #[test]
    fn write_mirror_creates_nested_path() {
        let dir = tempfile::tempdir().unwrap();
        write_mirror(dir.path(), "catalog/celestrak/active/latest/active.json", b"[]").unwrap();
        let p = dir.path().join("catalog/celestrak/active/latest/active.json");
        assert_eq!(std::fs::read(p).unwrap(), b"[]");
    }
}
