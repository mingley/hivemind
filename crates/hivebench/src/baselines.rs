//! Evaluation conditions / baselines from `docs/05-evaluation-plan.md`.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::Result;
use hivemind_core::{Actor, ClaimData, ClaimKind, Scope};
use hivemind_engine::{Durability, Options, Query, Remember, Store};
use serde::{Deserialize, Serialize};

use crate::scenario::{GoldClaim, Scenario};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Condition {
    /// B0: no persistent memory.
    B0,
    /// B1b: curated notes file always loaded.
    B1b,
    /// B1c: git-markdown memory with grep.
    B1c,
    /// B2: lexical retrieval over a hivemind store (BM25 stand-in).
    B2,
    /// Oracle: gold memories injected.
    Oracle,
    /// Hivemind with gold+distractors (given-memories mode).
    Hivemind,
}

impl Condition {
    pub fn id(self) -> &'static str {
        match self {
            Self::B0 => "B0",
            Self::B1b => "B1b",
            Self::B1c => "B1c",
            Self::B2 => "B2",
            Self::Oracle => "O",
            Self::Hivemind => "H",
        }
    }
}

/// Populate a store with gold (+ optional distractors) for given-memories mode.
/// Returns a map from gold id → engine claim id string.
pub fn ingest_given(
    store: &Store,
    scenario: &Scenario,
    include_distractors: bool,
) -> Result<BTreeMap<String, String>> {
    let actor = Actor::agent("hivebench");
    let mut map = BTreeMap::new();
    for claim in scenario.gold.iter().chain(
        include_distractors
            .then_some(scenario.distractors.iter())
            .into_iter()
            .flatten(),
    ) {
        let receipt = remember_gold(store, &actor, claim)?;
        if let Some(id) = receipt.claim {
            map.insert(claim.id.clone(), id.to_string());
        }
    }
    Ok(map)
}

fn remember_gold(
    store: &Store,
    actor: &Actor,
    claim: &GoldClaim,
) -> Result<hivemind_engine::Receipt> {
    let kind: ClaimKind = claim.kind.parse()?;
    let scope: Scope = claim.scope.parse()?;
    let mut data = ClaimData::new(kind, claim.statement.clone(), scope);
    data.about = claim.about.clone();
    data.applies_when = claim.when.clone();
    for (k, v) in &claim.fields {
        data.payload
            .insert(k.clone(), serde_json::Value::String(v.clone()));
    }
    let mut req = Remember::new(data);
    req.idempotency_key = Some(format!("gold:{}", claim.id));
    Ok(store.remember(actor, req)?)
}

/// Open an empty NoFsync store in `dir`.
pub fn open_store(dir: impl AsRef<Path>) -> Result<Store> {
    Ok(Store::open(
        dir,
        Options {
            durability: Durability::NoFsync,
            ..Options::default()
        },
    )?)
}

/// Render B1b notes file content from gold active claims.
pub fn notes_file(scenario: &Scenario) -> String {
    let mut out = String::from("# AGENTS.md / notes\n\n");
    for c in &scenario.gold {
        if c.status == "active" {
            out.push_str("- ");
            out.push_str(&c.statement);
            out.push('\n');
        }
    }
    out
}

/// Write a B1c-style MEMORY.md + topic files under `dir`.
pub fn write_markdown_memory(dir: impl AsRef<Path>, scenario: &Scenario) -> Result<()> {
    let dir = dir.as_ref();
    fs::create_dir_all(dir)?;
    let mut index = String::from("# MEMORY.md\n\n");
    for c in &scenario.gold {
        if c.status != "active" {
            continue;
        }
        let slug = c.id.replace(['/', ':'], "-");
        let path = dir.join(format!("{slug}.md"));
        fs::write(&path, format!("# {}\n\n{}\n", c.id, c.statement))?;
        index.push_str(&format!("- [{id}]({slug}.md)\n", id = c.id, slug = slug));
    }
    fs::write(dir.join("MEMORY.md"), index)?;
    Ok(())
}

/// Grep-like baseline: return gold claim ids whose statement/about contain all query tokens.
pub fn grep_baseline(scenario: &Scenario, query: &str, k: usize) -> Vec<String> {
    let tokens: Vec<_> = query
        .split_whitespace()
        .map(|t| t.to_ascii_lowercase())
        .filter(|t| t.len() > 2)
        .collect();
    let mut hits = Vec::new();
    for c in &scenario.gold {
        if c.status != "active" {
            continue;
        }
        let hay = format!("{} {}", c.statement, c.about.join(" ")).to_ascii_lowercase();
        if tokens.iter().all(|t| hay.contains(t)) {
            hits.push(c.id.clone());
        }
        if hits.len() >= k {
            break;
        }
    }
    hits
}

/// Build a recall [`Query`] from probe fields.
pub fn probe_query(
    text: &str,
    ctx: &BTreeMap<String, String>,
    include_history: bool,
    include_inapplicable: bool,
    k: usize,
) -> Query {
    Query {
        text: text.to_owned(),
        context: ctx.clone(),
        include_history,
        include_inapplicable,
        limit: Some(k),
        ..Query::default()
    }
}
