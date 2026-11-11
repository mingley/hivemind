//! Copilot JSONL and markdown import/export (M3).
//!
//! Copilot's exchange shape is `{scope, subject, fact, reason, citations}`
//! (`research/04`). Markdown export is a plain git-friendly projection.

use std::fmt::Write as _;
use std::io::{BufRead, Write};

use anyhow::{Result, anyhow, bail};
use hivemind_core::{
    Actor, Citation, ClaimData, ClaimId, ClaimKind, Method, Origin, Provenance, Scope,
};
use hivemind_engine::{ClaimDetail, Remember, Store};
use serde::{Deserialize, Serialize};

/// One Copilot memory JSONL record.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CopilotMemory {
    pub scope: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    pub fact: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub citations: Vec<CopilotCitation>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum CopilotCitation {
    Code {
        file: String,
        #[serde(rename = "lineStart", skip_serializing_if = "Option::is_none")]
        line_start: Option<u32>,
        #[serde(rename = "lineEnd", skip_serializing_if = "Option::is_none")]
        line_end: Option<u32>,
    },
    Text {
        text: String,
    },
}

impl From<&Citation> for CopilotCitation {
    fn from(c: &Citation) -> Self {
        CopilotCitation::Code {
            file: c.path.clone(),
            line_start: c.start_line,
            line_end: c.end_line,
        }
    }
}

impl CopilotCitation {
    fn to_citation(&self) -> Option<Citation> {
        match self {
            CopilotCitation::Code {
                file,
                line_start,
                line_end,
            } => Some(Citation {
                path: file.clone(),
                start_line: *line_start,
                end_line: *line_end,
                ..Citation::default()
            }),
            CopilotCitation::Text { .. } => None,
        }
    }

    fn to_quote(&self) -> Option<String> {
        match self {
            CopilotCitation::Text { text } => Some(text.clone()),
            CopilotCitation::Code { .. } => None,
        }
    }
}

impl CopilotMemory {
    pub fn from_detail(detail: &ClaimDetail) -> Self {
        let mut citations: Vec<CopilotCitation> = detail
            .provenance
            .citations
            .iter()
            .map(CopilotCitation::from)
            .collect();
        for evidence in &detail.provenance.evidence {
            if let hivemind_core::EvidenceRef::Quote(q) = evidence {
                citations.push(CopilotCitation::Text { text: q.clone() });
            }
        }
        Self {
            scope: detail.scope.to_string(),
            subject: detail.about.first().cloned(),
            fact: detail.statement.clone(),
            reason: detail
                .payload
                .get("reason")
                .and_then(|v| v.as_str())
                .map(str::to_owned),
            citations,
        }
    }

    pub fn into_claim_data(self) -> Result<ClaimData> {
        let scope: Scope = self
            .scope
            .parse()
            .map_err(|e| anyhow!("invalid scope `{}`: {e}", self.scope))?;
        let mut data = ClaimData::new(ClaimKind::Fact, self.fact, scope);
        if let Some(subject) = self.subject
            && !subject.trim().is_empty()
        {
            data.about.push(subject);
        }
        if let Some(reason) = self.reason
            && !reason.trim().is_empty()
        {
            data.payload
                .insert("reason".into(), serde_json::Value::String(reason));
        }
        let mut citations = Vec::new();
        let mut quotes = Vec::new();
        for c in self.citations {
            if let Some(cite) = c.to_citation() {
                citations.push(cite);
            } else if let Some(q) = c.to_quote() {
                quotes.push(hivemind_core::EvidenceRef::Quote(q));
            }
        }
        data.provenance = Provenance {
            method: Method::Imported,
            // Copilot JSONL is typically repo-scoped facts, not third-party web text.
            origin: Origin::RepoFile,
            evidence: quotes,
            citations,
            derived_from: Vec::new(),
        };
        Ok(data)
    }
}

/// Parse Copilot JSONL from a reader. Empty lines are skipped.
pub fn read_copilot_jsonl(reader: impl BufRead) -> Result<Vec<CopilotMemory>> {
    let mut out = Vec::new();
    for (i, line) in reader.lines().enumerate() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let record: CopilotMemory =
            serde_json::from_str(trimmed).map_err(|e| anyhow!("JSONL line {}: {e}", i + 1))?;
        out.push(record);
    }
    Ok(out)
}

pub fn write_copilot_jsonl(writer: &mut impl Write, memories: &[CopilotMemory]) -> Result<()> {
    for memory in memories {
        serde_json::to_writer(&mut *writer, memory)?;
        writer.write_all(b"\n")?;
    }
    Ok(())
}

/// Import Copilot JSONL records into the store. Returns claim ids written.
pub fn import_copilot_jsonl(
    store: &Store,
    actor: &Actor,
    memories: Vec<CopilotMemory>,
    dry_run: bool,
) -> Result<Vec<ClaimId>> {
    let mut ids = Vec::new();
    for memory in memories {
        let data = memory.into_claim_data()?;
        data.validate()
            .map_err(|e| anyhow!("invalid imported claim: {e}"))?;
        if dry_run {
            continue;
        }
        let receipt = store.remember(actor, Remember::new(data))?;
        if let Some(id) = receipt.claim {
            ids.push(id);
        }
    }
    Ok(ids)
}

/// Export active claims as Copilot JSONL.
pub fn export_copilot_jsonl(store: &Store, limit: usize) -> Result<Vec<CopilotMemory>> {
    let recall = store.recall(&hivemind_engine::Query {
        text: String::new(),
        include_history: false,
        limit: Some(limit),
        budget_tokens: Some(usize::MAX / 4),
        ..Default::default()
    });
    let mut out = Vec::new();
    for card in recall.cards {
        if let Some(detail) = store.get(&card.id, None) {
            out.push(CopilotMemory::from_detail(&detail));
        }
    }
    Ok(out)
}

/// Export claims as a simple markdown memory document.
pub fn export_markdown(store: &Store, limit: usize) -> Result<String> {
    let memories = export_copilot_jsonl(store, limit)?;
    let mut md = String::from("# Memory\n\n");
    for memory in memories {
        let _ = writeln!(md, "## {}\n", memory.fact);
        let _ = writeln!(md, "- **scope:** `{}`", memory.scope);
        if let Some(subject) = &memory.subject {
            let _ = writeln!(md, "- **subject:** `{subject}`");
        }
        if let Some(reason) = &memory.reason {
            let _ = writeln!(md, "- **reason:** {reason}");
        }
        for citation in &memory.citations {
            match citation {
                CopilotCitation::Code {
                    file,
                    line_start,
                    line_end,
                } => {
                    let lines = match (line_start, line_end) {
                        (Some(s), Some(e)) if s != e => format!(":{s}-{e}"),
                        (Some(s), _) => format!(":{s}"),
                        _ => String::new(),
                    };
                    let _ = writeln!(md, "- **cites:** `{file}{lines}`");
                }
                CopilotCitation::Text { text } => {
                    let _ = writeln!(md, "- **quote:** {text}");
                }
            }
        }
        md.push('\n');
    }
    Ok(md)
}

/// Import a markdown export produced by [`export_markdown`].
pub fn import_markdown(
    store: &Store,
    actor: &Actor,
    text: &str,
    dry_run: bool,
) -> Result<Vec<ClaimId>> {
    let mut memories = Vec::new();
    let mut current: Option<CopilotMemory> = None;
    for line in text.lines() {
        let line = line.trim_end();
        if let Some(title) = line.strip_prefix("## ") {
            if let Some(prev) = current.take() {
                memories.push(prev);
            }
            current = Some(CopilotMemory {
                scope: "global".into(),
                subject: None,
                fact: title.trim().to_owned(),
                reason: None,
                citations: Vec::new(),
            });
            continue;
        }
        let Some(mem) = current.as_mut() else {
            continue;
        };
        if let Some(rest) = line.strip_prefix("- **scope:**") {
            mem.scope = strip_ticks(rest.trim()).to_owned();
        } else if let Some(rest) = line.strip_prefix("- **subject:**") {
            mem.subject = Some(strip_ticks(rest.trim()).to_owned());
        } else if let Some(rest) = line.strip_prefix("- **reason:**") {
            mem.reason = Some(rest.trim().to_owned());
        } else if let Some(rest) = line.strip_prefix("- **cites:**") {
            let cite = strip_ticks(rest.trim());
            let parsed: Citation = cite
                .parse()
                .map_err(|e| anyhow!("bad citation `{cite}`: {e}"))?;
            mem.citations.push(CopilotCitation::from(&parsed));
        } else if let Some(rest) = line.strip_prefix("- **quote:**") {
            mem.citations.push(CopilotCitation::Text {
                text: rest.trim().to_owned(),
            });
        }
    }
    if let Some(prev) = current.take() {
        memories.push(prev);
    }
    if memories.is_empty() {
        bail!("no markdown memory sections found (expected `## fact` headings)");
    }
    import_copilot_jsonl(store, actor, memories, dry_run)
}

fn strip_ticks(s: &str) -> &str {
    s.trim()
        .strip_prefix('`')
        .and_then(|s| s.strip_suffix('`'))
        .unwrap_or(s.trim())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hivemind_engine::{Durability, Options};
    use tempfile::tempdir;

    #[test]
    fn jsonl_round_trip_through_store() {
        let dir = tempdir().unwrap();
        let store = Store::open(
            dir.path(),
            Options {
                durability: Durability::NoFsync,
                ..Options::default()
            },
        )
        .unwrap();
        let actor = Actor::agent("importer");
        let line = r#"{"scope":"project:acme/api","subject":"pkg:cargo/openssl-sys","fact":"Set OPENSSL_DIR on macOS","reason":"linker miss","citations":[{"file":"src/main.rs","lineStart":10,"lineEnd":12}]}"#;
        let memories = read_copilot_jsonl(line.as_bytes()).unwrap();
        let ids = import_copilot_jsonl(&store, &actor, memories, false).unwrap();
        assert_eq!(ids.len(), 1);
        let exported = export_copilot_jsonl(&store, 10).unwrap();
        assert_eq!(exported.len(), 1);
        assert_eq!(exported[0].fact, "Set OPENSSL_DIR on macOS");
        assert_eq!(
            exported[0].subject.as_deref(),
            Some("pkg:cargo/openssl-sys")
        );

        let md = export_markdown(&store, 10).unwrap();
        assert!(md.contains("## Set OPENSSL_DIR on macOS"));
        let ids2 = import_markdown(&store, &actor, &md, false).unwrap();
        assert_eq!(ids2.len(), 1);
    }
}
