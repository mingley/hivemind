//! Filesystem helpers that make metadata changes durable.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;

use crate::error::{Error, Result};

/// Extra free space required beyond the payload itself before a write proceeds.
pub(crate) const DISK_SAFETY_MARGIN: u64 = 1024 * 1024;

/// Flushes a directory so that entries created or renamed in it survive power
/// loss. On macOS, Rust's `sync_all` issues `F_FULLFSYNC`.
pub(crate) fn sync_dir(dir: &Path) -> io::Result<()> {
    File::open(dir)?.sync_all()
}

/// Replaces `path` with `bytes` atomically: write a temporary file in the same
/// directory, flush it, rename it over `path`, then flush the directory so the
/// rename itself is durable. Readers see either the old or the new content.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8], sync: bool) -> io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| io::Error::other("path has no parent directory"))?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| io::Error::other("path has no file name"))?;
    let tmp = dir.join(format!(".{name}.tmp-{}", uuid::Uuid::now_v7().simple()));
    let result = (|| {
        let mut file = File::create(&tmp)?;
        file.write_all(bytes)?;
        if sync {
            file.sync_all()?;
        }
        fs::rename(&tmp, path)?;
        if sync {
            sync_dir(dir)?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Free bytes available to non-privileged writers on the filesystem that holds
/// `path` (`f_bavail * f_frsize` from `statvfs`).
pub(crate) fn available_space(path: &Path) -> Result<u64> {
    #[cfg(test)]
    if let Some(forced) = free_bytes_override() {
        return Ok(forced);
    }
    let stat = rustix::fs::statvfs(path).map_err(io::Error::from)?;
    Ok(stat.f_bavail.saturating_mul(stat.f_frsize))
}

/// Ensures the filesystem holding `path` has at least
/// [`DISK_SAFETY_MARGIN`] + `payload` free bytes before a write.
pub(crate) fn ensure_disk_space(path: &Path, payload: u64) -> Result<()> {
    let needed = DISK_SAFETY_MARGIN.saturating_add(payload);
    let available = available_space(path)?;
    if available < needed {
        return Err(Error::DiskFull { available, needed });
    }
    Ok(())
}

#[cfg(test)]
mod free_space_override {
    use std::cell::Cell;

    thread_local! {
        static FREE_BYTES: Cell<Option<u64>> = const { Cell::new(None) };
    }

    pub(super) fn get() -> Option<u64> {
        FREE_BYTES.with(Cell::get)
    }

    /// Forces [`super::available_space`] for the duration of `f`. Nested calls
    /// restore the previous override.
    pub(crate) fn with<T>(free: Option<u64>, f: impl FnOnce() -> T) -> T {
        FREE_BYTES.with(|cell| {
            let previous = cell.replace(free);
            let out = f();
            cell.set(previous);
            out
        })
    }
}

#[cfg(test)]
fn free_bytes_override() -> Option<u64> {
    free_space_override::get()
}

#[cfg(test)]
pub(crate) use free_space_override::with as with_free_bytes;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_when_below_margin_plus_payload() {
        let tmp = tempfile::tempdir().unwrap();
        with_free_bytes(Some(DISK_SAFETY_MARGIN), || {
            let err = ensure_disk_space(tmp.path(), 1).unwrap_err();
            assert!(
                matches!(
                    err,
                    Error::DiskFull {
                        available: a,
                        needed: n
                    } if a == DISK_SAFETY_MARGIN && n == DISK_SAFETY_MARGIN + 1
                ),
                "{err}"
            );
        });
    }

    #[test]
    fn allows_when_margin_and_payload_fit() {
        let tmp = tempfile::tempdir().unwrap();
        with_free_bytes(Some(DISK_SAFETY_MARGIN + 64), || {
            ensure_disk_space(tmp.path(), 64).unwrap();
        });
    }

    #[test]
    fn reports_real_free_space_on_this_filesystem() {
        let tmp = tempfile::tempdir().unwrap();
        let free = available_space(tmp.path()).unwrap();
        assert!(free > 0, "expected some free space, got {free}");
    }
}
