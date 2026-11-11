//! Read-time citation verification against a workspace tree.
//!
//! A citation that carries a `content_hash` can be checked cheaply against the
//! reader's files. Status is one of `valid`, `drifted`, or `broken`
//! (`docs/03-memory-model.md`).

use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::Serialize;

use crate::claim::Citation;

/// Prefix for content hashes produced by [`hash_cited_text`].
pub const CONTENT_HASH_PREFIX: &str = "b3:";

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CitationStatus {
    /// Cited bytes match the stored hash.
    Valid,
    /// The file exists but the cited region no longer matches.
    Drifted,
    /// The file or lines are missing, or the path is unsafe.
    Broken,
    /// No hash to check, or no workspace was provided.
    Unchecked,
}

impl CitationStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            CitationStatus::Valid => "valid",
            CitationStatus::Drifted => "drifted",
            CitationStatus::Broken => "broken",
            CitationStatus::Unchecked => "unchecked",
        }
    }
}

impl std::fmt::Display for CitationStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
pub struct CitationCheck {
    pub citation: String,
    pub status: CitationStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl CitationCheck {
    pub fn is_stale(&self) -> bool {
        matches!(
            self.status,
            CitationStatus::Drifted | CitationStatus::Broken
        )
    }
}

/// BLAKE3 of the cited text, hex-encoded with a `b3:` prefix.
pub fn hash_cited_text(text: &str) -> String {
    let digest = blake3::hash(text.as_bytes());
    format!("{CONTENT_HASH_PREFIX}{}", digest.to_hex())
}

/// Extract the cited region from file contents. Line numbers are 1-based and
/// inclusive. Without a line range, the whole file is used.
pub fn extract_cited_text(
    contents: &str,
    start_line: Option<u32>,
    end_line: Option<u32>,
) -> Result<String, String> {
    let normalized = contents.replace("\r\n", "\n");
    let lines: Vec<&str> = normalized.split('\n').collect();
    match (start_line, end_line) {
        (None, None) => Ok(normalized),
        (Some(start), end) => {
            let start = start.max(1) as usize;
            let end = end.unwrap_or(start as u32).max(start as u32) as usize;
            if start > lines.len() {
                return Err(format!(
                    "start line {start} is past end of file ({} lines)",
                    lines.len()
                ));
            }
            let end = end.min(lines.len());
            Ok(lines[start - 1..end].join("\n"))
        }
        (None, Some(_)) => Err("end_line without start_line".into()),
    }
}

/// Resolve a citation path under `root`, refusing absolute paths and `..`.
pub fn resolve_citation_path(root: &Path, citation: &Citation) -> Result<PathBuf, String> {
    let path = Path::new(&citation.path);
    if path.is_absolute() {
        return Err("citation path must be relative to the workspace".into());
    }
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err("citation path must not contain `..`".into());
    }
    Ok(root.join(path))
}

/// Hash the cited region of a file on disk so writers can store `content_hash`.
pub fn hash_file_citation(root: &Path, citation: &Citation) -> Result<String, String> {
    let path = resolve_citation_path(root, citation)?;
    let contents =
        fs::read_to_string(&path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    let text = extract_cited_text(&contents, citation.start_line, citation.end_line)?;
    Ok(hash_cited_text(&text))
}

/// Verify one citation against files under `root`.
pub fn verify_citation(root: &Path, citation: &Citation) -> CitationCheck {
    let key = citation.to_string();
    let Some(expected) = citation.content_hash.as_ref() else {
        return CitationCheck {
            citation: key,
            status: CitationStatus::Unchecked,
            expected_hash: None,
            actual_hash: None,
            detail: Some("no content_hash on citation".into()),
        };
    };
    let path = match resolve_citation_path(root, citation) {
        Ok(p) => p,
        Err(detail) => {
            return CitationCheck {
                citation: key,
                status: CitationStatus::Broken,
                expected_hash: Some(expected.clone()),
                actual_hash: None,
                detail: Some(detail),
            };
        }
    };
    let contents = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return CitationCheck {
                citation: key,
                status: CitationStatus::Broken,
                expected_hash: Some(expected.clone()),
                actual_hash: None,
                detail: Some(format!("missing file {}", path.display())),
            };
        }
        Err(e) => {
            return CitationCheck {
                citation: key,
                status: CitationStatus::Broken,
                expected_hash: Some(expected.clone()),
                actual_hash: None,
                detail: Some(format!("reading {}: {e}", path.display())),
            };
        }
    };
    let text = match extract_cited_text(&contents, citation.start_line, citation.end_line) {
        Ok(t) => t,
        Err(detail) => {
            return CitationCheck {
                citation: key,
                status: CitationStatus::Broken,
                expected_hash: Some(expected.clone()),
                actual_hash: None,
                detail: Some(detail),
            };
        }
    };
    let actual = hash_cited_text(&text);
    if actual == *expected {
        CitationCheck {
            citation: key,
            status: CitationStatus::Valid,
            expected_hash: Some(expected.clone()),
            actual_hash: Some(actual),
            detail: None,
        }
    } else {
        CitationCheck {
            citation: key,
            status: CitationStatus::Drifted,
            expected_hash: Some(expected.clone()),
            actual_hash: Some(actual),
            detail: Some("cited content changed".into()),
        }
    }
}

/// Verify every citation that has a content hash.
pub fn verify_citations(root: &Path, citations: &[Citation]) -> Vec<CitationCheck> {
    citations.iter().map(|c| verify_citation(root, c)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn detects_valid_drifted_and_broken() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("src/main.rs");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, "fn a() {}\nfn b() {}\nfn c() {}\n").unwrap();

        let mut cite = Citation {
            path: "src/main.rs".into(),
            start_line: Some(2),
            end_line: Some(2),
            ..Citation::default()
        };
        cite.content_hash = Some(hash_file_citation(dir.path(), &cite).unwrap());
        assert_eq!(
            verify_citation(dir.path(), &cite).status,
            CitationStatus::Valid
        );

        fs::write(&file, "fn a() {}\nfn b_changed() {}\nfn c() {}\n").unwrap();
        assert_eq!(
            verify_citation(dir.path(), &cite).status,
            CitationStatus::Drifted
        );

        cite.path = "src/missing.rs".into();
        assert_eq!(
            verify_citation(dir.path(), &cite).status,
            CitationStatus::Broken
        );
    }

    #[test]
    fn rejects_parent_dir_paths() {
        let err = resolve_citation_path(
            Path::new("/tmp"),
            &Citation {
                path: "../etc/passwd".into(),
                ..Citation::default()
            },
        )
        .unwrap_err();
        assert!(err.contains(".."));
    }
}
