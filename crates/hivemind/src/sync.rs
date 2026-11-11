//! Laptop ↔ service sync helpers (M5).
//!
//! - **Pull:** fetch `changes_since` from a remote cursor into a local JSONL file.
//! - **Upload queue:** append-only JSONL of remember payloads; flush by POSTing
//!   each line to the service (idempotency keys make retries safe).

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UploadItem {
    pub idempotency_key: String,
    pub payload: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PullCursor {
    pub tenant: String,
    pub next_offset: u64,
}

pub fn queue_path(store_dir: &Path) -> PathBuf {
    store_dir.join("sync").join("upload_queue.jsonl")
}

pub fn cursor_path(store_dir: &Path) -> PathBuf {
    store_dir.join("sync").join("pull_cursor.json")
}

pub fn enqueue(store_dir: &Path, item: &UploadItem) -> Result<()> {
    let path = queue_path(store_dir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("open {}", path.display()))?;
    serde_json::to_writer(&mut file, item)?;
    file.write_all(b"\n")?;
    Ok(())
}

pub fn read_queue(store_dir: &Path) -> Result<Vec<UploadItem>> {
    let path = queue_path(store_dir);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = File::open(&path)?;
    let mut out = Vec::new();
    for (i, line) in BufReader::new(file).lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let item: UploadItem =
            serde_json::from_str(&line).with_context(|| format!("upload queue line {}", i + 1))?;
        out.push(item);
    }
    Ok(out)
}

/// Replace the queue with `remaining` (call after a successful flush prefix).
pub fn rewrite_queue(store_dir: &Path, remaining: &[UploadItem]) -> Result<()> {
    let path = queue_path(store_dir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("jsonl.tmp");
    {
        let mut file = File::create(&tmp)?;
        for item in remaining {
            serde_json::to_writer(&mut file, item)?;
            file.write_all(b"\n")?;
        }
        file.sync_all()?;
    }
    fs::rename(&tmp, &path)?;
    Ok(())
}

pub fn load_cursor(store_dir: &Path) -> Result<Option<PullCursor>> {
    let path = cursor_path(store_dir);
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(&path)?;
    Ok(Some(serde_json::from_slice(&bytes)?))
}

pub fn save_cursor(store_dir: &Path, cursor: &PullCursor) -> Result<()> {
    let path = cursor_path(store_dir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(cursor)?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, &bytes)?;
    fs::rename(&tmp, &path)?;
    Ok(())
}

pub fn append_pulled_events(store_dir: &Path, events: &[Value]) -> Result<PathBuf> {
    let path = store_dir.join("sync").join("pulled.jsonl");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
    for event in events {
        serde_json::to_writer(&mut file, event)?;
        file.write_all(b"\n")?;
    }
    Ok(path)
}

/// Parse `Authorization: Bearer <token>`.
pub fn bearer_token<'a>(header: Option<&'a str>) -> Result<&'a str> {
    let Some(header) = header else {
        bail!("missing Authorization header");
    };
    let token = header
        .strip_prefix("Bearer ")
        .or_else(|| header.strip_prefix("bearer "))
        .ok_or_else(|| anyhow::anyhow!("Authorization must be Bearer <token>"))?
        .trim();
    if token.is_empty() {
        bail!("empty bearer token");
    }
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    #[test]
    fn queue_round_trip_and_cursor() {
        let dir = tempdir().unwrap();
        enqueue(
            dir.path(),
            &UploadItem {
                idempotency_key: "k1".into(),
                payload: json!({"statement": "hi"}),
            },
        )
        .unwrap();
        let q = read_queue(dir.path()).unwrap();
        assert_eq!(q.len(), 1);
        rewrite_queue(dir.path(), &[]).unwrap();
        assert!(read_queue(dir.path()).unwrap().is_empty());

        save_cursor(
            dir.path(),
            &PullCursor {
                tenant: "acme".into(),
                next_offset: 3,
            },
        )
        .unwrap();
        let c = load_cursor(dir.path()).unwrap().unwrap();
        assert_eq!(c.next_offset, 3);
    }
}
