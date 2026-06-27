//! Persistence on the mounted volume: status.json (the change-detection source of
//! truth) and a mirror of every product's latest bytes under its R2 key path.

use std::path::Path;

use anyhow::{Context, Result};
use fs4::FileExt;

use crate::status::{parse_status, serialize_status, Status};

const STATUS_FILE: &str = "status.json";
const LOCK_FILE: &str = ".ssdm.lock";

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

/// Acquire an exclusive advisory lock on `<data_dir>/.ssdm.lock`.
///
/// Returns the open [`std::fs::File`]; the caller MUST keep it alive for the
/// duration of the process — dropping it releases the lock.  On contention,
/// returns an error with a human-readable message so the operator knows why
/// the process refused to start.
pub fn acquire_lock(data_dir: &Path) -> Result<std::fs::File> {
    std::fs::create_dir_all(data_dir).with_context(|| format!("create {data_dir:?}"))?;
    let path = data_dir.join(LOCK_FILE);
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .open(&path)
        .with_context(|| format!("open lock file {path:?}"))?;
    match <std::fs::File as FileExt>::try_lock(&file) {
        Ok(()) => Ok(file),
        // Contention: another process holds the lock — refuse with a clear message.
        Err(fs4::TryLockError::WouldBlock) => anyhow::bail!(
            "another ssdm process holds the lock on {}; \
             manual `sync` cannot run while the daemon is running",
            path.display()
        ),
        // A genuine IO/syscall error (permissions, disk, etc.): surface it.
        Err(fs4::TryLockError::Error(e)) => {
            Err(e).with_context(|| format!("lock {}", path.display()))
        }
    }
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
    fn acquire_lock_exclusive_and_releases_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        // First acquire succeeds.
        let lock1 = acquire_lock(dir.path()).expect("first acquire must succeed");
        // Second acquire on the same dir must fail while the first is held.
        let err = acquire_lock(dir.path()).expect_err("second acquire must fail");
        assert!(
            err.to_string().contains("another ssdm process"),
            "error message must explain contention: {err}"
        );
        // After dropping the first lock, a third acquire must succeed.
        drop(lock1);
        acquire_lock(dir.path()).expect("third acquire after drop must succeed");
    }

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
