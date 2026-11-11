//! Token ledger: every attributable model call is tagged for cost accounting.
//!
//! Categories follow `docs/05-evaluation-plan.md`.

use serde::{Deserialize, Serialize};

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
}
