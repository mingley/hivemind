//! Controlled agent loop with real token accounting.
//!
//! Runs each scenario probe as a turn: charges measured tool-schema tokens,
//! runs the condition's memory path, and records retrieved context from
//! concrete card / notes / grep text. Embedding categories are estimated from
//! input text length (chars÷4) without calling the embeddings worker.
//!
//! See `docs/hivebench-ledger.md` for measured vs estimated categories and
//! how to run the ledger CLI.

use std::collections::BTreeMap;

use serde::Serialize;
use tempfile::TempDir;

use crate::baselines::{self, Condition};
use crate::er::{ErMetrics, run_er};
use crate::ledger::{
    ActorKind, Category, CountSource, Ledger, LedgerEntry, estimate_tokens,
};
use crate::scenario::{Probe, Scenario};
use crate::tools::tool_schema_tokens;
use hivemind_engine::{Store, render_card};

#[derive(Clone, Debug, Serialize)]
pub struct RunRecord {
    pub scenario: String,
    pub condition: String,
    pub er: ErMetrics,
    pub ledger: Ledger,
    /// Number of controlled-loop turns (one per probe).
    pub turns: usize,
}

#[derive(Clone, Debug)]
pub struct LoopOptions {
    pub run_id: String,
    /// Expand the top recalled card to L1 detail (follow-up expansion).
    pub expand_top: bool,
    pub k: usize,
}

impl Default for LoopOptions {
    fn default() -> Self {
        Self {
            run_id: "controlled".into(),
            expand_top: true,
            k: 8,
        }
    }
}

/// Controlled loop: offline E-R plus per-turn memory-attributable token accounting.
pub fn run_controlled(
    scenario: &Scenario,
    condition: Condition,
    opts: LoopOptions,
) -> anyhow::Result<RunRecord> {
    let er = run_er(scenario, condition, opts.k)?;
    let mut ledger = Ledger::default();
    let meta = EntryMeta {
        run: opts.run_id.clone(),
        scenario: scenario.meta.id.clone(),
        condition: condition.id().into(),
    };

    // Write-path: given-memories ingest → write embeddings (estimated from statements).
    record_write_path(&mut ledger, &meta, scenario, condition);

    let store_tmp = if matches!(condition, Condition::Hivemind | Condition::B2) {
        let tmp = TempDir::new()?;
        let store = baselines::open_store(tmp.path())?;
        let include_distractors = matches!(condition, Condition::Hivemind | Condition::B2);
        baselines::ingest_given(&store, scenario, include_distractors)?;
        Some((tmp, store))
    } else {
        None
    };

    let schema_per_turn = tool_schema_tokens(condition);

    for probe in &scenario.probes {
        if schema_per_turn > 0 {
            push(
                &mut ledger,
                &meta,
                Charge {
                    actor: ActorKind::Harness,
                    category: Category::ToolSchema,
                    input_tokens: schema_per_turn,
                    source: CountSource::Measured,
                    note: format!("tools/list for {} (turn {})", condition.id(), probe.id),
                },
            );
        }

        // Query formulation only when the condition actually issues a memory query.
        if !matches!(condition, Condition::B0) {
            let q_tokens = estimate_tokens(&probe.query);
            if q_tokens > 0 {
                push(
                    &mut ledger,
                    &meta,
                    Charge {
                        actor: ActorKind::Agent,
                        category: Category::QueryFormulation,
                        input_tokens: q_tokens,
                        source: CountSource::Measured,
                        note: format!("probe {} query text", probe.id),
                    },
                );
            }
        }

        match condition {
            Condition::B0 => {
                // No memory tools or retrieval; zero memory-attributable tokens.
            }
            Condition::B1b => {
                let notes = baselines::notes_file(scenario);
                let tokens = estimate_tokens(&notes);
                push(
                    &mut ledger,
                    &meta,
                    Charge {
                        actor: ActorKind::Harness,
                        category: Category::RetrievedContext,
                        input_tokens: tokens,
                        source: CountSource::Measured,
                        note: format!("B1b notes file (turn {})", probe.id),
                    },
                );
            }
            Condition::B1c => {
                let hits = baselines::grep_baseline(scenario, &probe.query, opts.k);
                let mut body = String::new();
                for id in &hits {
                    if let Some(g) = scenario.gold.iter().find(|g| &g.id == id) {
                        body.push_str(&format!("# {}\n{}\n\n", g.id, g.statement));
                    }
                }
                let tokens = estimate_tokens(&body);
                if tokens > 0 {
                    push(
                        &mut ledger,
                        &meta,
                        Charge {
                            actor: ActorKind::Harness,
                            category: Category::RetrievedContext,
                            input_tokens: tokens,
                            source: CountSource::Measured,
                            note: format!("B1c grep hits (turn {})", probe.id),
                        },
                    );
                }
            }
            Condition::Oracle => {
                let body = oracle_context(scenario, probe);
                let tokens = estimate_tokens(&body);
                if tokens > 0 {
                    push(
                        &mut ledger,
                        &meta,
                        Charge {
                            actor: ActorKind::Harness,
                            category: Category::RetrievedContext,
                            input_tokens: tokens,
                            source: CountSource::Measured,
                            note: format!("oracle gold inject (turn {})", probe.id),
                        },
                    );
                }
            }
            Condition::B2 | Condition::Hivemind => {
                let store = &store_tmp.as_ref().expect("store for H/B2").1;
                record_store_turn(&mut ledger, &meta, store, probe, opts.k, opts.expand_top)?;
            }
        }
    }

    Ok(RunRecord {
        scenario: scenario.meta.id.clone(),
        condition: condition.id().into(),
        er,
        turns: scenario.probes.len(),
        ledger,
    })
}

/// Backward-compatible name: controlled loop with default options.
pub fn dry_run(scenario: &Scenario, condition: Condition, k: usize) -> anyhow::Result<RunRecord> {
    run_controlled(
        scenario,
        condition,
        LoopOptions {
            run_id: "controlled".into(),
            expand_top: true,
            k,
        },
    )
}

struct EntryMeta {
    run: String,
    scenario: String,
    condition: String,
}

struct Charge {
    actor: ActorKind,
    category: Category,
    input_tokens: u64,
    source: CountSource,
    note: String,
}

fn push(ledger: &mut Ledger, meta: &EntryMeta, charge: Charge) {
    ledger.record(LedgerEntry {
        run: meta.run.clone(),
        scenario: meta.scenario.clone(),
        condition: meta.condition.clone(),
        actor: charge.actor,
        category: charge.category,
        input_tokens: charge.input_tokens,
        output_tokens: 0,
        cached_input_tokens: 0,
        source: charge.source,
        note: charge.note,
    });
}

fn record_write_path(
    ledger: &mut Ledger,
    meta: &EntryMeta,
    scenario: &Scenario,
    condition: Condition,
) {
    match condition {
        Condition::B0 | Condition::Oracle => {}
        Condition::B1b | Condition::B1c => {
            // Notes / markdown are authored offline; charge write embedding on
            // active gold statements as the cost of materializing that corpus.
            for claim in scenario.gold.iter().filter(|c| c.status == "active") {
                let n = estimate_tokens(&claim.statement);
                if n == 0 {
                    continue;
                }
                push(
                    ledger,
                    meta,
                    Charge {
                        actor: ActorKind::Embedder,
                        category: Category::WriteEmbedding,
                        input_tokens: n,
                        source: CountSource::Estimated,
                        note: format!("write embed {}", claim.id),
                    },
                );
            }
        }
        Condition::B2 | Condition::Hivemind => {
            let claims = scenario.gold.iter().chain(scenario.distractors.iter());
            for claim in claims {
                let n = estimate_tokens(&claim.statement);
                if n == 0 {
                    continue;
                }
                push(
                    ledger,
                    meta,
                    Charge {
                        actor: ActorKind::Embedder,
                        category: Category::WriteEmbedding,
                        input_tokens: n,
                        source: CountSource::Estimated,
                        note: format!(
                            "write embed {} (no vector worker; chars÷4 of statement)",
                            claim.id
                        ),
                    },
                );
            }
        }
    }
}

fn oracle_context(scenario: &Scenario, probe: &Probe) -> String {
    if probe.expect.abstain {
        return String::new();
    }
    let mut body = String::new();
    for id in &probe.expect.claim_ids {
        if let Some(g) = scenario.gold.iter().find(|g| &g.id == id) {
            body.push_str(&format!("[{}] {}\n", g.id, g.statement));
        }
    }
    body
}

fn record_store_turn(
    ledger: &mut Ledger,
    meta: &EntryMeta,
    store: &Store,
    probe: &Probe,
    k: usize,
    expand_top: bool,
) -> anyhow::Result<()> {
    let limit = probe.k.unwrap_or(k);

    // Query embedding: estimated from query text (does not call vector.rs).
    let q_embed = estimate_tokens(&probe.query);
    if q_embed > 0 {
        push(
            ledger,
            meta,
            Charge {
                actor: ActorKind::Embedder,
                category: Category::QueryEmbedding,
                input_tokens: q_embed,
                source: CountSource::Estimated,
                note: format!(
                    "query embed probe {} (no vector worker; chars÷4)",
                    probe.id
                ),
            },
        );
    }

    let query = baselines::probe_query(
        &probe.query,
        &probe.ctx,
        probe.include_history,
        probe.include_inapplicable,
        limit,
    );
    let recall = store.recall(&query);

    let mut rendered = String::new();
    for card in &recall.cards {
        rendered.push_str(&render_card(card));
        rendered.push('\n');
    }
    let measured = estimate_tokens(&rendered);
    // Prefer concrete render; cross-check engine budget estimate when present.
    let tokens = if measured > 0 {
        measured
    } else {
        recall.estimated_tokens as u64
    };
    if tokens > 0 {
        push(
            ledger,
            meta,
            Charge {
                actor: ActorKind::Harness,
                category: Category::RetrievedContext,
                input_tokens: tokens,
                source: CountSource::Measured,
                note: format!(
                    "L0 cards probe {} (engine estimated_tokens={})",
                    probe.id, recall.estimated_tokens
                ),
            },
        );
    }

    if expand_top
        && let Some(top) = recall.cards.first()
        && let Some(detail) = store.get(&top.id, Some(&probe.ctx))
    {
        let json = serde_json::to_string(&detail)?;
        let n = estimate_tokens(&json);
        if n > 0 {
            push(
                ledger,
                meta,
                Charge {
                    actor: ActorKind::Agent,
                    category: Category::FollowupExpansion,
                    input_tokens: n,
                    source: CountSource::Measured,
                    note: format!("L1 show {} (probe {})", top.id, probe.id),
                },
            );
        }
    }

    Ok(())
}

/// Convenience: category totals for JSON reports.
pub fn category_totals(ledger: &Ledger) -> BTreeMap<String, u64> {
    ledger.tokens_by_category()
}
