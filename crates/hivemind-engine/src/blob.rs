//! Content-addressed blob store for artifacts (logs, diffs, transcripts).
//!
//! Blobs live at `blobs/<first two hex digits>/<remaining hex>` under their
//! BLAKE3 hash. A blob is written to a temporary file, flushed, renamed into
//! place, and its directory flushed, so it is durable before any event refers
//! to it. Reads re-hash the content, so silent corruption is detected.

use std::fs;
use std::path::{Path, PathBuf};

use hivemind_core::BlobHash;

use crate::error::{Error, Result};
use crate::fsutil;
use crate::log::Durability;

#[derive(Debug)]
pub struct BlobStore {
    root: PathBuf,
    sync: bool,
}

impl BlobStore {
    pub fn open(root: PathBuf, durability: Durability) -> Result<Self> {
        fs::create_dir_all(&root)?;
        Ok(Self {
            root,
            sync: durability == Durability::Fsync,
        })
    }

    fn path(&self, hash: &BlobHash) -> PathBuf {
        let hex = hash.to_hex();
        self.root.join(&hex[..2]).join(&hex[2..])
    }

    /// Stores `bytes` and returns its hash, and whether it was newly written.
    pub fn put(&self, bytes: &[u8]) -> Result<(BlobHash, bool)> {
        let hash = BlobHash::of(bytes);
        let path = self.path(&hash);
        if path.exists() {
            return Ok((hash, false));
        }
        let dir = path.parent().expect("blob paths have a parent");
        if !dir.exists() {
            fs::create_dir_all(dir)?;
            if self.sync {
                fsutil::sync_dir(&self.root)?;
            }
        }
        fsutil::write_atomic(&path, bytes, self.sync)?;
        Ok((hash, true))
    }

    pub fn get(&self, hash: &BlobHash) -> Result<Vec<u8>> {
        let path = self.path(hash);
        let bytes = fs::read(&path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => Error::NotFound(format!("artifact {hash}")),
            _ => Error::Io(e),
        })?;
        check(&path, hash, &bytes)?;
        Ok(bytes)
    }

    pub fn contains(&self, hash: &BlobHash) -> bool {
        self.path(hash).exists()
    }

    /// Re-hashes every blob. Returns the number checked and any problems found.
    pub fn verify_all(&self) -> Result<(usize, Vec<String>)> {
        let mut checked = 0;
        let mut problems = Vec::new();
        for shard in fs::read_dir(&self.root)? {
            let shard = shard?;
            if !shard.file_type()?.is_dir() {
                continue;
            }
            let prefix = shard.file_name().to_string_lossy().into_owned();
            for entry in fs::read_dir(shard.path())? {
                let entry = entry?;
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') {
                    problems.push(format!("stray temporary file {}", entry.path().display()));
                    continue;
                }
                let Ok(hash) = format!("{prefix}{name}").parse::<BlobHash>() else {
                    problems.push(format!("unexpected file {}", entry.path().display()));
                    continue;
                };
                checked += 1;
                let bytes = fs::read(entry.path())?;
                if let Err(e) = check(&entry.path(), &hash, &bytes) {
                    problems.push(e.to_string());
                }
            }
        }
        Ok((checked, problems))
    }
}

fn check(path: &Path, hash: &BlobHash, bytes: &[u8]) -> Result<()> {
    if BlobHash::of(bytes) == *hash {
        Ok(())
    } else {
        Err(Error::corruption(
            path,
            0,
            format!("content does not match {hash}"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_deduplicates_and_detects_corruption() {
        let tmp = tempfile::tempdir().unwrap();
        let blobs = BlobStore::open(tmp.path().to_path_buf(), Durability::Fsync).unwrap();
        let (hash, fresh) = blobs.put(b"build log").unwrap();
        assert!(fresh);
        assert_eq!(blobs.put(b"build log").unwrap(), (hash, false));
        assert_eq!(blobs.get(&hash).unwrap(), b"build log");
        assert_eq!(blobs.verify_all().unwrap(), (1, vec![]));

        fs::write(blobs.path(&hash), b"tampered").unwrap();
        assert!(matches!(blobs.get(&hash), Err(Error::Corruption { .. })));
        assert_eq!(blobs.verify_all().unwrap().1.len(), 1);

        let missing = BlobHash::of(b"never stored");
        assert!(matches!(blobs.get(&missing), Err(Error::NotFound(_))));
    }
}
