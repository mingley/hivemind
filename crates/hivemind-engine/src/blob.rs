//! Content-addressed blob store for artifacts (logs, diffs, transcripts).
//!
//! Blobs live at `blobs/<first two hex digits>/<remaining hex>` under their
//! BLAKE3 hash. A blob is written to `blobs/tmp/<random>`, flushed, renamed
//! into place, and its directory flushed, so it is durable before any event
//! refers to it. Leftover files in `blobs/tmp/` are swept on open. Reads
//! re-hash the content, so silent corruption is detected.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use hivemind_core::BlobHash;

use crate::error::{Error, Result};
use crate::fsutil;
use crate::log::Durability;

const TMP_DIR: &str = "tmp";

#[derive(Debug)]
pub struct BlobStore {
    root: PathBuf,
    sync: bool,
}

impl BlobStore {
    pub fn open(root: PathBuf, durability: Durability) -> Result<Self> {
        fs::create_dir_all(&root)?;
        let tmp = root.join(TMP_DIR);
        fs::create_dir_all(&tmp)?;
        sweep_tmp(&tmp)?;
        Ok(Self {
            root,
            sync: durability == Durability::Fsync,
        })
    }

    fn tmp_dir(&self) -> PathBuf {
        self.root.join(TMP_DIR)
    }

    fn path(&self, hash: &BlobHash) -> PathBuf {
        let hex = hash.to_hex();
        self.root.join(&hex[..2]).join(&hex[2..])
    }

    /// Stores `bytes` and returns its hash, and whether it was newly written.
    /// On return the blob and both directory levels above it are durable, even
    /// when the blob already existed: another writer's rename may be visible
    /// before its directory flush has finished.
    pub fn put(&self, bytes: &[u8]) -> Result<(BlobHash, bool)> {
        let hash = BlobHash::of(bytes);
        let path = self.path(&hash);
        let shard = path.parent().expect("blob paths have a parent");
        let fresh = !path.exists();
        if fresh {
            fsutil::ensure_disk_space(&self.root, bytes.len() as u64)?;
            fs::create_dir_all(shard)?;
            let tmp = self
                .tmp_dir()
                .join(uuid::Uuid::now_v7().simple().to_string());
            let result = (|| -> Result<()> {
                let mut file = File::create(&tmp)?;
                file.write_all(bytes)?;
                if self.sync {
                    file.sync_all()?;
                }
                fs::rename(&tmp, &path)?;
                if self.sync {
                    fsutil::sync_dir(shard)?;
                }
                Ok(())
            })();
            if result.is_err() {
                let _ = fs::remove_file(&tmp);
            }
            result?;
        }
        if self.sync {
            if !fresh {
                fsutil::sync_dir(shard)?;
            }
            fsutil::sync_dir(&self.root)?;
        }
        Ok((hash, fresh))
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
            // Staging directory for in-flight writes; not a content shard.
            if prefix == TMP_DIR {
                continue;
            }
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

/// Removes leftover files from an interrupted blob put. Best-effort: a missing
/// `tmp/` is fine; failures listing or removing entries surface as errors.
fn sweep_tmp(tmp: &Path) -> Result<()> {
    for entry in fs::read_dir(tmp)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            fs::remove_dir_all(&path)?;
        } else {
            fs::remove_file(&path)?;
        }
    }
    Ok(())
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
    use crate::fsutil::{DISK_SAFETY_MARGIN, with_free_bytes};

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

    #[test]
    fn open_sweeps_stray_files_in_tmp() {
        let root = tempfile::tempdir().unwrap();
        let tmp = root.path().join(TMP_DIR);
        fs::create_dir_all(&tmp).unwrap();
        fs::write(tmp.join("leftover-a"), b"partial").unwrap();
        fs::create_dir_all(tmp.join("nested")).unwrap();
        fs::write(tmp.join("nested").join("leftover-b"), b"more").unwrap();

        let blobs = BlobStore::open(root.path().to_path_buf(), Durability::NoFsync).unwrap();
        assert!(
            fs::read_dir(blobs.tmp_dir()).unwrap().next().is_none(),
            "tmp/ should be empty after open"
        );
        // A real put still works and leaves no staging residue.
        blobs.put(b"ok").unwrap();
        assert!(fs::read_dir(blobs.tmp_dir()).unwrap().next().is_none());
    }

    #[test]
    fn put_refuses_when_disk_is_full() {
        let root = tempfile::tempdir().unwrap();
        let blobs = BlobStore::open(root.path().to_path_buf(), Durability::NoFsync).unwrap();
        let payload = vec![0u8; 128];
        with_free_bytes(Some(DISK_SAFETY_MARGIN), || {
            let err = blobs.put(&payload).unwrap_err();
            assert!(
                matches!(err, Error::DiskFull { available, needed }
                    if available == DISK_SAFETY_MARGIN
                        && needed == DISK_SAFETY_MARGIN + payload.len() as u64),
                "{err}"
            );
        });
        // No staged leftovers and no shard content after a refused put.
        assert!(fs::read_dir(blobs.tmp_dir()).unwrap().next().is_none());
        assert_eq!(blobs.verify_all().unwrap(), (0, vec![]));
    }

    #[test]
    fn duplicate_put_skips_disk_space_check() {
        let root = tempfile::tempdir().unwrap();
        let blobs = BlobStore::open(root.path().to_path_buf(), Durability::NoFsync).unwrap();
        let (hash, _) = blobs.put(b"already there").unwrap();
        with_free_bytes(Some(0), || {
            assert_eq!(blobs.put(b"already there").unwrap(), (hash, false));
        });
    }
}
