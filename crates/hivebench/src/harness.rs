//! Controlled agent harness skeleton with a token ledger.
//!
//! Full LLM agent loops are opt-in later; this module defines the run record
//! shape and a dry-run path that exercises memory conditions without an API.

use serde::Serialize;

use crate::baselines::Condition;
use crate::er::{ErMetrics, run_er};
use crate::ledger::{ActorKind, Category, Ledger, LedgerEntry};
use crate::scenario::Scenario;

#[derive(Clone, Debug, Serialize)]
pub struct RunRecord {
    pub scenario: String,
    pub condition: String,
    pub er: ErMetrics,
    pub ledger: Ledger,
}

/// Dry-run a scenario under a condition: offline E-R plus a ledger stub that
/// records tool-schema tokens for memory tools (condition H / B2).
pub fn dry_run(scenario: &Scenario, condition: Condition, k: usize) -> anyhow::Result<RunRecord> {
    let er = run_er(scenario, condition, k)?;
    let mut ledger = Ledger::default();
    let schema_tokens = match condition {
        Condition::Hivemind | Condition::B2 => 800, // six MCP tool schemas, approximate
        Condition::B1b => 200,
        Condition::B1c => 150,
        Condition::Oracle => 100,
        Condition::B0 => 0,
    };
    if schema_tokens > 0 {
        ledger.record(LedgerEntry {
            run: "dry".into(),
            scenario: scenario.meta.id.clone(),
            condition: condition.id().into(),
            actor: ActorKind::Harness,
            category: Category::ToolSchema,
            input_tokens: schema_tokens,
            output_tokens: 0,
            cached_input_tokens: 0,
            note: "approximate tool/notes schema tokens".into(),
        });
    }
    // Retrieved context estimate: ~60 tokens per card * hits
    let retrieved = (er.hits_at_k as u64).saturating_mul(60);
    if retrieved > 0 {
        ledger.record(LedgerEntry {
            run: "dry".into(),
            scenario: scenario.meta.id.clone(),
            condition: condition.id().into(),
            actor: ActorKind::Harness,
            category: Category::RetrievedContext,
            input_tokens: retrieved,
            output_tokens: 0,
            cached_input_tokens: 0,
            note: "estimated from offline hits".into(),
        });
    }
    Ok(RunRecord {
        scenario: scenario.meta.id.clone(),
        condition: condition.id().into(),
        er,
        ledger,
    })
}
