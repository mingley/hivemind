//! Projection checkpoints (M6): a sealed event-prefix pack at a log offset.
//!
//! A snapshot records every acknowledged event with `offset < next_offset`,
//! hashed with BLAKE3. On open, the store applies the pack into projections and
//! then replays only newer log events (the log is still fully validated).
//!
//! Layout (`snapshots/<next_offset:020>.hsnap`):
//! ```text
//! magic "HMSNAP\0\x01" | format u32 | next_offset u64 | event_count u64
//! | payload_len u64 | blake3(payload) [32] | payload
//! payload: event_count × (offset u64 | len u32 | bytes[len])
//! ```

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{Error, Result};
use crate::fsutil;
use crate::log::Durability;

pub const SNAPSHOT_MAGIC: [u8; 8] = *b"HMSNAP\0\x01";
pub const SNAPSHOT_FORMAT: u32 = 1;
const HEADER_LEN: usize = 8 + 4 + 8 + 8 + 8 + 32;

#[derive(Clone, Debug, Serialize)]
pub struct SnapshotInfo {
    pub path: PathBuf,
    pub next_offset: u64,
    pub events: u64,
    pub bytes: u64,
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub next_offset: u64,
    pub events: Vec<(u64, Vec<u8>)>,
}

/// Write a snapshot covering `events` whose offsets are contiguous from 0 and
/// whose next offset would be `next_offset`.
pub fn write(
    snapshots_dir: &Path,
    next_offset: u64,
    events: &[(u64, Vec<u8>)],
    durability: Durability,
) -> Result<SnapshotInfo> {
    if next_offset as usize != events.len() {
        return Err(Error::Invalid(format!(
            "snapshot next_offset {next_offset} does not match {} events",
            events.len()
        )));
    }
    for (i, (offset, _)) in events.iter().enumerate() {
        if *offset != i as u64 {
            return Err(Error::Invalid(format!(
                "snapshot events must be contiguous from 0; saw offset {offset} at index {i}"
            )));
        }
    }

    fs::create_dir_all(snapshots_dir)?;
    let name = format!("{next_offset:020}.hsnap");
    let final_path = snapshots_dir.join(&name);
    let tmp_path = snapshots_dir.join(format!("{name}.tmp"));

    let mut payload = Vec::new();
    for (offset, bytes) in events {
        if bytes.len() > u32::MAX as usize {
            return Err(Error::Invalid("snapshot event too large".into()));
        }
        payload.extend_from_slice(&offset.to_le_bytes());
        payload.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        payload.extend_from_slice(bytes);
    }
    let digest = blake3::hash(&payload);

    let mut header = Vec::with_capacity(HEADER_LEN);
    header.extend_from_slice(&SNAPSHOT_MAGIC);
    header.extend_from_slice(&SNAPSHOT_FORMAT.to_le_bytes());
    header.extend_from_slice(&next_offset.to_le_bytes());
    header.extend_from_slice(&(events.len() as u64).to_le_bytes());
    header.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    header.extend_from_slice(digest.as_bytes());

    {
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&tmp_path)?;
        file.write_all(&header)?;
        file.write_all(&payload)?;
        if durability == Durability::Fsync {
            file.sync_all()?;
        }
    }
    fs::rename(&tmp_path, &final_path)?;
    if durability == Durability::Fsync {
        fsutil::sync_dir(snapshots_dir)?;
    }

    Ok(SnapshotInfo {
        path: final_path,
        next_offset,
        events: events.len() as u64,
        bytes: (HEADER_LEN + payload.len()) as u64,
    })
}

/// Load the newest readable snapshot under `snapshots_dir`, if any.
pub fn load_latest(snapshots_dir: &Path) -> Result<Option<Snapshot>> {
    if !snapshots_dir.exists() {
        return Ok(None);
    }
    let mut paths: Vec<PathBuf> = fs::read_dir(snapshots_dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e == "hsnap")
        })
        .collect();
    paths.sort();
    while let Some(path) = paths.pop() {
        match read_file(&path) {
            Ok(snap) => return Ok(Some(snap)),
            Err(Error::Corruption { .. } | Error::Format(_)) => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(None)
}

fn read_file(path: &Path) -> Result<Snapshot> {
    let mut file = File::open(path)?;
    let mut header = [0u8; HEADER_LEN];
    file.read_exact(&mut header)?;
    if header[0..8] != SNAPSHOT_MAGIC {
        return Err(Error::Format(format!(
            "{}: bad snapshot magic",
            path.display()
        )));
    }
    let format = u32::from_le_bytes(header[8..12].try_into().unwrap());
    if format != SNAPSHOT_FORMAT {
        return Err(Error::Format(format!(
            "{}: unsupported snapshot format {format}",
            path.display()
        )));
    }
    let next_offset = u64::from_le_bytes(header[12..20].try_into().unwrap());
    let event_count = u64::from_le_bytes(header[20..28].try_into().unwrap());
    let payload_len = u64::from_le_bytes(header[28..36].try_into().unwrap()) as usize;
    let expected_hash: [u8; 32] = header[36..68].try_into().unwrap();

    let mut payload = vec![0u8; payload_len];
    file.read_exact(&mut payload)?;
    let digest = blake3::hash(&payload);
    if digest.as_bytes() != &expected_hash {
        return Err(Error::corruption(
            path,
            HEADER_LEN as u64,
            "snapshot payload blake3 mismatch",
        ));
    }

    let mut events = Vec::with_capacity(event_count as usize);
    let mut cursor = 0usize;
    for i in 0..event_count {
        if cursor + 12 > payload.len() {
            return Err(Error::corruption(
                path,
                (HEADER_LEN + cursor) as u64,
                "truncated snapshot payload",
            ));
        }
        let offset = u64::from_le_bytes(payload[cursor..cursor + 8].try_into().unwrap());
        cursor += 8;
        let len = u32::from_le_bytes(payload[cursor..cursor + 4].try_into().unwrap()) as usize;
        cursor += 4;
        if offset != i {
            return Err(Error::corruption(
                path,
                (HEADER_LEN + cursor) as u64,
                format!("snapshot event offset {offset} != {i}"),
            ));
        }
        if cursor + len > payload.len() {
            return Err(Error::corruption(
                path,
                (HEADER_LEN + cursor) as u64,
                "truncated snapshot event",
            ));
        }
        events.push((offset, payload[cursor..cursor + len].to_vec()));
        cursor += len;
    }
    if cursor != payload.len() {
        return Err(Error::corruption(
            path,
            (HEADER_LEN + cursor) as u64,
            "trailing bytes in snapshot payload",
        ));
    }
    if next_offset != event_count {
        return Err(Error::Format(format!(
            "{}: next_offset {next_offset} != event_count {event_count}",
            path.display()
        )));
    }
    Ok(Snapshot {
        next_offset,
        events,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn write_load_round_trip() {
        let dir = tempdir().unwrap();
        let events = vec![
            (0, b"alpha".to_vec()),
            (1, b"beta".to_vec()),
            (2, b"gamma".to_vec()),
        ];
        let info = write(dir.path(), 3, &events, Durability::NoFsync).unwrap();
        assert_eq!(info.events, 3);
        let loaded = load_latest(dir.path()).unwrap().unwrap();
        assert_eq!(loaded.next_offset, 3);
        assert_eq!(loaded.events, events);
    }

    #[test]
    fn rejects_tampered_payload() {
        let dir = tempdir().unwrap();
        let events = vec![(0, b"x".to_vec())];
        let info = write(dir.path(), 1, &events, Durability::NoFsync).unwrap();
        let mut bytes = fs::read(&info.path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        fs::write(&info.path, bytes).unwrap();
        assert!(load_latest(dir.path()).unwrap().is_none());
    }
}
