//! Filesystem helpers that make metadata changes durable.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;

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
