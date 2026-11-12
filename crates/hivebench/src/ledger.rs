//! Token ledger: every attributable model call is tagged for cost accounting.
//!
//! Categories follow `docs/05-evaluation-plan.md`. Counts are either
//! **measured** from concrete text (schemas, rendered cards, queries) or
//! **estimated** with the same chars÷4 heuristic the engine uses for budgets
//! (see [`estimate_tokens`]). Embedding categories are estimated from input
//! text length; they do not invoke the embeddings worker.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Same rough budget estimator as `hivemind_engine::recall::estimate_tokens`.
pub fn estimate_tokens(text: &str) -> u64 {
    text.chars().count().div_ceil(4) as u64
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorKind {
    Agent,
    MemoryWriter,
    Consolidator,
    Embedder,
    Judge,
    Harness,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Extraction,
    DedupUpdate,
    Consolidation,
    QueryFormulation,
    QueryEmbedding,
    WriteEmbedding,
    RetrievedContext,
    FollowupExpansion,
    ToolSchema,
    AgentOther,
    Background,
    Judge,
}

/// Whether a ledger count was taken from concrete text or a faithful estimate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CountSource {
    /// Counted from concrete serialized text (schemas, cards, prompts).
    #[default]
    Measured,
    /// Faithful estimate (chars÷4) where no real billed tokenizer/API exists
    /// in this harness — notably embedding input size without calling vector.rs.
    Estimated,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub run: String,
    pub scenario: String,
    pub condition: String,
    pub actor: ActorKind,
    pub category: Category,
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(default)]
    pub cached_input_tokens: u64,
    #[serde(default)]
    pub source: CountSource,
    #[serde(default)]
    pub note: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Ledger {
    pub entries: Vec<LedgerEntry>,
}

impl Ledger {
    pub fn record(&mut self, entry: LedgerEntry) {
        self.entries.push(entry);
    }

    pub fn total_non_judge_tokens(&self) -> u64 {
        self.entries
            .iter()
            .filter(|e| e.category != Category::Judge && e.actor != ActorKind::Judge)
            .map(|e| e.input_tokens + e.output_tokens)
            .sum()
    }

    pub fn memory_attributable_tokens(&self) -> u64 {
        use Category::*;
        self.entries
            .iter()
            .filter(|e| {
                matches!(
                    e.category,
                    Extraction
                        | DedupUpdate
                        | Consolidation
                        | QueryFormulation
                        | QueryEmbedding
                        | WriteEmbedding
                        | RetrievedContext
                        | FollowupExpansion
                        | ToolSchema
                        | Background
                )
            })
            .map(|e| e.input_tokens + e.output_tokens)
            .sum()
    }

    pub fn tokens_by_category(&self) -> BTreeMap<String, u64> {
        let mut map = BTreeMap::new();
        for e in &self.entries {
            let key = serde_json::to_value(e.category)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_else(|| format!("{:?}", e.category));
            *map.entry(key).or_default() += e.input_tokens + e.output_tokens;
        }
        map
    }

    pub fn measured_tokens(&self) -> u64 {
        self.entries
            .iter()
            .filter(|e| e.source == CountSource::Measured)
            .map(|e| e.input_tokens + e.output_tokens)
            .sum()
    }

    pub fn estimated_tokens(&self) -> u64 {
        self.entries
            .iter()
            .filter(|e| e.source == CountSource::Estimated)
            .map(|e| e.input_tokens + e.output_tokens)
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_tokens_matches_engine_heuristic() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcde"), 2);
        assert_eq!(
            estimate_tokens("abcd"),
            hivemind_engine::recall::estimate_tokens("abcd") as u64
        );
    }

    #[test]
    fn memory_attributable_excludes_agent_other_and_judge() {
        let mut ledger = Ledger::default();
        ledger.record(LedgerEntry {
            run: "t".into(),
            scenario: "s".into(),
            condition: "H".into(),
            actor: ActorKind::Harness,
            category: Category::ToolSchema,
            input_tokens: 10,
            output_tokens: 0,
            cached_input_tokens: 0,
            source: CountSource::Measured,
            note: String::new(),
        });
        ledger.record(LedgerEntry {
            run: "t".into(),
            scenario: "s".into(),
            condition: "H".into(),
            actor: ActorKind::Agent,
            category: Category::AgentOther,
            input_tokens: 99,
            output_tokens: 0,
            cached_input_tokens: 0,
            source: CountSource::Measured,
            note: String::new(),
        });
        ledger.record(LedgerEntry {
            run: "t".into(),
            scenario: "s".into(),
            condition: "H".into(),
            actor: ActorKind::Judge,
            category: Category::Judge,
            input_tokens: 50,
            output_tokens: 0,
            cached_input_tokens: 0,
            source: CountSource::Measured,
            note: String::new(),
        });
        assert_eq!(ledger.memory_attributable_tokens(), 10);
        assert_eq!(ledger.total_non_judge_tokens(), 109);
    }
}
