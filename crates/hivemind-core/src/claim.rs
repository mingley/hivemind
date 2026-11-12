//! Claims: the atomic unit of reusable knowledge.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::ids::{BlobHash, ClaimId};
use crate::scope::Scope;
use crate::time::Timestamp;

/// Longest accepted statement, in characters. Claims are meant to be small;
/// details belong in the payload and evidence.
pub const MAX_STATEMENT_CHARS: usize = 4_000;
const MAX_LIST_LEN: usize = 64;
const MAX_KEY_CHARS: usize = 128;
const MAX_ENTITY_CHARS: usize = 256;
const MAX_QUOTE_BYTES: usize = 2_048;
const MAX_PAYLOAD_BYTES: usize = 64 * 1024;
/// Deepest nesting accepted in the payload. Event decoding has a recursion
/// limit, so anything stored must stay well inside it to replay.
const MAX_PAYLOAD_DEPTH: usize = 32;

str_enum! {
    pub enum ClaimKind {
        Fact => "fact",
        Decision => "decision",
        Preference => "preference",
        Procedure => "procedure",
        /// Problem, root cause, fix, and dead ends.
        Solution => "solution",
        /// A gotcha or "don't do X".
        Warning => "warning",
        /// An open question, answered later by a linked claim.
        Question => "question",
        /// Structured state of partially completed work.
        Handoff => "handoff",
    }
}

str_enum! {
    /// How the claim was produced. Recorded so the cost and reliability of each
    /// write path can be measured separately.
    pub enum Method {
        AgentAuthored => "agent_authored",
        LlmExtracted => "llm_extracted",
        Human => "human",
        Imported => "imported",
        Consolidated => "consolidated",
    }
}

str_enum! {
    /// Where the claim's content came from. `External` content (web pages, issue
    /// comments, third-party text in tool output) is untrusted, and everything
    /// derived from it inherits that taint.
    pub enum Origin {
        Agent => "agent",
        Human => "human",
        ToolOutput => "tool_output",
        RepoFile => "repo_file",
        External => "external",
    }
}

/// A pointer to evidence that an agent can inspect on demand.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceRef {
    /// An artifact in the blob store (logs, diffs, transcripts).
    Artifact(BlobHash),
    Url(String),
    /// A short verbatim excerpt, such as one error line.
    Quote(String),
    /// Another claim, such as a decision record.
    Claim(ClaimId),
}

impl EvidenceRef {
    /// Stable key used to detect shared evidence ("repetition is not corroboration").
    pub fn key(&self) -> String {
        match self {
            EvidenceRef::Artifact(h) => h.to_string(),
            EvidenceRef::Url(u) => format!("url:{u}"),
            EvidenceRef::Quote(q) => format!("quote:{}", BlobHash::of(q.as_bytes()).to_hex()),
            EvidenceRef::Claim(c) => c.to_string(),
        }
    }
}

/// A verifiable pointer into code: `path[:start[-end]][@commit]`.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Default, Serialize, Deserialize)]
pub struct Citation {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_line: Option<u32>,
    /// Hash of the cited lines at `commit`, used to detect drift on read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
}

impl Citation {
    pub fn key(&self) -> String {
        self.to_string()
    }
}

impl fmt::Display for Citation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(repo) = &self.repo {
            write!(f, "{repo}/")?;
        }
        f.write_str(&self.path)?;
        match (self.start_line, self.end_line) {
            (Some(s), Some(e)) if e != s => write!(f, ":{s}-{e}")?,
            (Some(s), _) => write!(f, ":{s}")?,
            _ => {}
        }
        if let Some(commit) = &self.commit {
            write!(f, "@{commit}")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid citation `{0}`; expected path[:start[-end]][@commit]")]
pub struct CitationParseError(pub String);

impl FromStr for Citation {
    type Err = CitationParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || CitationParseError(s.to_owned());
        let (rest, commit) = match s.rsplit_once('@') {
            Some((rest, commit)) if !commit.is_empty() => (rest, Some(commit.to_owned())),
            Some(_) => return Err(err()),
            None => (s, None),
        };
        let (path, start_line, end_line) = match rest.rsplit_once(':') {
            Some((path, lines)) if lines.chars().all(|c| c.is_ascii_digit() || c == '-') => {
                let (start, end) = match lines.split_once('-') {
                    Some((a, b)) => (a, b),
                    None => (lines, lines),
                };
                let start: u32 = start.parse().map_err(|_| err())?;
                let end: u32 = end.parse().map_err(|_| err())?;
                if end < start {
                    return Err(err());
                }
                (path, Some(start), Some(end))
            }
            _ => (rest, None, None),
        };
        if path.is_empty() {
            return Err(err());
        }
        Ok(Citation {
            path: path.to_owned(),
            commit,
            start_line,
            end_line,
            ..Citation::default()
        })
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Provenance {
    pub method: Method,
    pub origin: Origin,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<EvidenceRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub citations: Vec<Citation>,
    /// Claims this one was derived from. Trust never exceeds the weakest input.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub derived_from: Vec<ClaimId>,
}

impl Default for Provenance {
    fn default() -> Self {
        Self {
            method: Method::AgentAuthored,
            origin: Origin::Agent,
            evidence: Vec::new(),
            citations: Vec::new(),
            derived_from: Vec::new(),
        }
    }
}

impl Provenance {
    /// Whether the claim points at anything inspectable.
    pub fn has_evidence(&self) -> bool {
        !self.evidence.is_empty() || !self.citations.is_empty()
    }

    /// Keys identifying the evidence this claim rests on.
    pub fn evidence_keys(&self) -> Vec<String> {
        self.evidence
            .iter()
            .map(EvidenceRef::key)
            .chain(self.citations.iter().map(Citation::key))
            .collect()
    }
}

/// Everything a writer asserts about a claim. Lifecycle state (status, record
/// time, belief) is assigned by the store, never by the writer.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct ClaimData {
    pub kind: ClaimKind,
    /// One or two sentences written for a future reader.
    pub statement: String,
    pub scope: Scope,
    /// Canonical keys of the entities this claim is about, such as
    /// `repo:github.com/acme/api` or `pkg:cargo/openssl-sys`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub about: Vec<String>,
    /// Structured fields that depend on the kind (error signature, fix, rationale).
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub payload: serde_json::Map<String, serde_json::Value>,
    /// Conditions under which the claim holds, evaluated against the reader's context.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub applies_when: BTreeMap<String, String>,
    /// When the claim became true in the world, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_from: Option<Timestamp>,
    /// When the claim stopped being true in the world, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_to: Option<Timestamp>,
    /// Review hint: unconfirmed after this time, the claim is flagged as expired.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_by: Option<Timestamp>,
    #[serde(default)]
    pub provenance: Provenance,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid claim: {0}")]
pub struct ValidationError(pub String);

impl ClaimData {
    pub fn new(kind: ClaimKind, statement: impl Into<String>, scope: Scope) -> Self {
        Self {
            kind,
            statement: statement.into(),
            scope,
            about: Vec::new(),
            payload: serde_json::Map::new(),
            applies_when: BTreeMap::new(),
            valid_from: None,
            valid_to: None,
            review_by: None,
            provenance: Provenance::default(),
        }
    }

    /// Stub left in the log after [`crate::EventBody::ClaimForgotten`] purges a
    /// claim's payload. Keeps the claim id's assertion event decodable without
    /// retaining statement, evidence, or structured fields.
    pub fn forgotten_stub(kind: ClaimKind) -> Self {
        Self::new(kind, "[forgotten]", Scope::Global)
    }

    /// Text indexed for lexical search: the statement, entity keys, and string
    /// values in the payload.
    pub fn index_text(&self) -> String {
        let mut text = self.statement.clone();
        for entity in &self.about {
            text.push('\n');
            text.push_str(entity);
        }
        for (key, value) in &self.payload {
            collect_strings(key, value, &mut text);
        }
        text
    }

    /// Every user-supplied string, for hygiene checks such as secret detection.
    pub fn strings(&self) -> Vec<&str> {
        let mut out: Vec<&str> = vec![self.statement.as_str()];
        out.extend(self.about.iter().map(String::as_str));
        out.extend(
            self.applies_when
                .iter()
                .flat_map(|(k, v)| [k.as_str(), v.as_str()]),
        );
        for value in self.payload.values() {
            push_json_strings(value, &mut out);
        }
        for evidence in &self.provenance.evidence {
            if let EvidenceRef::Quote(q) | EvidenceRef::Url(q) = evidence {
                out.push(q);
            }
        }
        out
    }

    pub fn validate(&self) -> Result<(), ValidationError> {
        let fail = |msg: String| Err(ValidationError(msg));
        if let Err(e) = self.scope.validate() {
            return fail(e.to_string());
        }
        for (name, time) in [
            ("valid_from", self.valid_from),
            ("valid_to", self.valid_to),
            ("review_by", self.review_by),
        ] {
            if let Some(t) = time
                && !t.is_representable()
            {
                return fail(format!("{name} must fall between 1970 and the year 9999"));
            }
        }
        if self
            .payload
            .values()
            .any(|v| json_depth(v) >= MAX_PAYLOAD_DEPTH)
        {
            return fail(format!(
                "payload nests deeper than {MAX_PAYLOAD_DEPTH} levels"
            ));
        }
        let statement_chars = self.statement.chars().count();
        if self.statement.trim().is_empty() {
            return fail("statement is empty".into());
        }
        if statement_chars > MAX_STATEMENT_CHARS {
            return fail(format!(
                "statement is {statement_chars} characters; the limit is {MAX_STATEMENT_CHARS}. Keep claims small and move details to the payload or evidence"
            ));
        }
        for (name, len) in [
            ("about", self.about.len()),
            ("applies_when", self.applies_when.len()),
            ("evidence", self.provenance.evidence.len()),
            ("citations", self.provenance.citations.len()),
            ("derived_from", self.provenance.derived_from.len()),
        ] {
            if len > MAX_LIST_LEN {
                return fail(format!(
                    "{name} has {len} entries; the limit is {MAX_LIST_LEN}"
                ));
            }
        }
        if let Some(entity) = self
            .about
            .iter()
            .find(|e| e.trim().is_empty() || e.chars().count() > MAX_ENTITY_CHARS)
        {
            return fail(format!("entity key `{entity}` is empty or too long"));
        }
        if let Some(key) = self.applies_when.keys().find(|k| !valid_condition_key(k)) {
            return fail(format!(
                "condition key `{key}` must be 1-{MAX_KEY_CHARS} characters of a-z, 0-9, and _ . : / @ + -"
            ));
        }
        for evidence in &self.provenance.evidence {
            if let EvidenceRef::Quote(q) = evidence
                && q.len() > MAX_QUOTE_BYTES
            {
                return fail(format!(
                    "quoted evidence is {} bytes; the limit is {MAX_QUOTE_BYTES}. Store long evidence as an artifact",
                    q.len()
                ));
            }
        }
        let payload_bytes = serde_json::to_vec(&self.payload).map_or(0, |v| v.len());
        if payload_bytes > MAX_PAYLOAD_BYTES {
            return fail(format!(
                "payload is {payload_bytes} bytes; the limit is {MAX_PAYLOAD_BYTES}"
            ));
        }
        if let (Some(from), Some(to)) = (self.valid_from, self.valid_to)
            && to < from
        {
            return fail("valid_to is before valid_from".into());
        }
        if self.kind == ClaimKind::Handoff {
            for key in ["goal", "done", "remaining", "next_step"] {
                match self.payload.get(key) {
                    Some(serde_json::Value::String(s)) if !s.trim().is_empty() => {}
                    Some(serde_json::Value::Array(items)) if !items.is_empty() => {}
                    _ => {
                        return fail(format!(
                            "handoff claims require non-empty payload fields goal, done, remaining, and next_step (missing or empty: {key})"
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

fn json_depth(value: &serde_json::Value) -> usize {
    match value {
        serde_json::Value::Array(items) => 1 + items.iter().map(json_depth).max().unwrap_or(0),
        serde_json::Value::Object(map) => 1 + map.values().map(json_depth).max().unwrap_or(0),
        _ => 0,
    }
}

fn valid_condition_key(key: &str) -> bool {
    !key.is_empty()
        && key.chars().count() <= MAX_KEY_CHARS
        && key
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "_.:/@+-".contains(c))
}

fn collect_strings(key: &str, value: &serde_json::Value, out: &mut String) {
    match value {
        serde_json::Value::String(s) => {
            out.push('\n');
            out.push_str(key);
            out.push(' ');
            out.push_str(s);
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_strings(key, item, out);
            }
        }
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                collect_strings(k, v, out);
            }
        }
        _ => {}
    }
}

fn push_json_strings<'a>(value: &'a serde_json::Value, out: &mut Vec<&'a str>) {
    match value {
        serde_json::Value::String(s) => out.push(s),
        serde_json::Value::Array(items) => items.iter().for_each(|v| push_json_strings(v, out)),
        serde_json::Value::Object(map) => map.values().for_each(|v| push_json_strings(v, out)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope() -> Scope {
        "project:acme/api".parse().unwrap()
    }

    #[test]
    fn parses_citations() {
        let c: Citation = "src/main.rs:10-20@4b2c9e1".parse().unwrap();
        assert_eq!(c.path, "src/main.rs");
        assert_eq!((c.start_line, c.end_line), (Some(10), Some(20)));
        assert_eq!(c.commit.as_deref(), Some("4b2c9e1"));
        assert_eq!(c.to_string(), "src/main.rs:10-20@4b2c9e1");

        let single: Citation = "build.rs:7".parse().unwrap();
        assert_eq!(single.to_string(), "build.rs:7");
        let bare: Citation = "Cargo.toml".parse().unwrap();
        assert_eq!(bare.start_line, None);
        assert!("src/a.rs:9-3".parse::<Citation>().is_err());
        assert!(":3".parse::<Citation>().is_err());
    }

    #[test]
    fn handoffs_require_structured_payload() {
        let mut claim = ClaimData::new(ClaimKind::Handoff, "pick up limiter work", scope());
        assert!(claim.validate().is_err());
        claim
            .payload
            .insert("goal".into(), serde_json::json!("finish limiter"));
        claim
            .payload
            .insert("done".into(), serde_json::json!(["skeleton"]));
        claim
            .payload
            .insert("remaining".into(), serde_json::json!(["tests"]));
        claim
            .payload
            .insert("next_step".into(), serde_json::json!("write failing test"));
        assert!(claim.validate().is_ok());
    }

    #[test]
    fn validates_limits() {
        let mut claim = ClaimData::new(ClaimKind::Fact, "  ", scope());
        assert!(claim.validate().is_err());
        claim.statement = "x".repeat(MAX_STATEMENT_CHARS + 1);
        assert!(claim.validate().is_err());
        claim.statement = "Integration tests need Docker".into();
        assert!(claim.validate().is_ok());
        claim.applies_when.insert("OS".into(), "macos".into());
        assert!(claim.validate().is_err());
    }

    #[test]
    fn rejects_values_that_would_not_replay() {
        let mut claim = ClaimData::new(
            ClaimKind::Fact,
            "x",
            Scope::Project("My Projects/app".into()),
        );
        assert!(claim.validate().is_err(), "scope with whitespace");
        claim.scope = scope();
        let mut deep = serde_json::json!("leaf");
        for _ in 0..40 {
            deep = serde_json::json!([deep]);
        }
        claim.payload.insert("deep".into(), deep);
        assert!(claim.validate().is_err(), "deep payload");
        claim.payload.clear();
        claim.review_by = Some(Timestamp::from_millis(i64::MAX));
        assert!(claim.validate().is_err(), "unrepresentable time");
    }

    #[test]
    fn index_text_includes_payload_and_entities() {
        let mut claim = ClaimData::new(ClaimKind::Solution, "Linking fails", scope());
        claim.about.push("pkg:cargo/openssl-sys".into());
        claim
            .payload
            .insert("fix".into(), serde_json::json!("export OPENSSL_DIR"));
        let text = claim.index_text();
        assert!(text.contains("openssl-sys"));
        assert!(text.contains("OPENSSL_DIR"));
    }

    #[test]
    fn evidence_keys_identify_shared_evidence() {
        let a = EvidenceRef::Quote("error[E0277]".into());
        let b = EvidenceRef::Quote("error[E0277]".into());
        assert_eq!(a.key(), b.key());
    }
}
