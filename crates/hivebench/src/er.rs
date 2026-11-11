//! E-R: offline retrieval evaluation (no LLM).

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use hivemind_core::Status;
use hivemind_core::applicability::Applicability;
use hivemind_engine::Flag;
use serde::Serialize;
use tempfile::TempDir;

use crate::baselines::{self, Condition};
use crate::scenario::{Probe, Scenario};

#[derive(Clone, Debug, Default, Serialize)]
pub struct ErMetrics {
    pub scenario: String,
    pub condition: String,
    pub probes: usize,
    pub hits_at_k: usize,
    pub expected_hits: usize,
    pub stale_hits: usize,
    pub ranked: usize,
    pub abstain_correct: usize,
    pub abstain_total: usize,
    pub applicability_ok: usize,
    pub applicability_checked: usize,
}

impl ErMetrics {
    pub fn recall_at_k(&self) -> f64 {
        if self.expected_hits == 0 {
            1.0
        } else {
            self.hits_at_k as f64 / self.expected_hits as f64
        }
    }

    pub fn stale_hit_rate(&self) -> f64 {
        if self.ranked == 0 {
            0.0
        } else {
            self.stale_hits as f64 / self.ranked as f64
        }
    }

    pub fn abstention_accuracy(&self) -> f64 {
        if self.abstain_total == 0 {
            1.0
        } else {
            self.abstain_correct as f64 / self.abstain_total as f64
        }
    }

    pub fn applicability_precision(&self) -> f64 {
        if self.applicability_checked == 0 {
            1.0
        } else {
            self.applicability_ok as f64 / self.applicability_checked as f64
        }
    }
}

/// Run offline retrieval for one scenario under a condition.
pub fn run_er(scenario: &Scenario, condition: Condition, k: usize) -> Result<ErMetrics> {
    let mut metrics = ErMetrics {
        scenario: scenario.meta.id.clone(),
        condition: condition.id().into(),
        ..ErMetrics::default()
    };

    match condition {
        Condition::B0 => {
            for probe in &scenario.probes {
                score_b0(&mut metrics, probe);
            }
            return Ok(metrics);
        }
        Condition::B1b | Condition::Oracle => {
            for probe in &scenario.probes {
                let ids = if condition == Condition::Oracle {
                    if probe.expect.abstain {
                        Vec::new()
                    } else {
                        probe.expect.claim_ids.clone()
                    }
                } else {
                    baselines::grep_baseline(scenario, &probe.query, k)
                };
                score_id_list(&mut metrics, scenario, probe, &ids, k);
            }
            return Ok(metrics);
        }
        Condition::B1c => {
            for probe in &scenario.probes {
                let ids = baselines::grep_baseline(scenario, &probe.query, k);
                score_id_list(&mut metrics, scenario, probe, &ids, k);
            }
            return Ok(metrics);
        }
        Condition::B2 | Condition::Hivemind => {}
    }

    let tmp = TempDir::new()?;
    let store = baselines::open_store(tmp.path())?;
    let include_distractors = matches!(condition, Condition::Hivemind | Condition::B2);
    let gold_map = baselines::ingest_given(&store, scenario, include_distractors)?;
    let claim_to_gold: BTreeMap<String, String> =
        gold_map.into_iter().map(|(g, c)| (c, g)).collect();

    for probe in &scenario.probes {
        let limit = probe.k.unwrap_or(k);
        let query = baselines::probe_query(
            &probe.query,
            &probe.ctx,
            probe.include_history,
            probe.include_inapplicable,
            limit,
        );
        let recall = store.recall(&query);
        let mut ids = Vec::new();
        for card in &recall.cards {
            metrics.ranked += 1;
            if card.status != Status::Active
                || card
                    .flags
                    .iter()
                    .any(|f| matches!(f, Flag::Historical | Flag::Inapplicable))
            {
                metrics.stale_hits += 1;
            }
            if !probe.ctx.is_empty() {
                metrics.applicability_checked += 1;
                if !matches!(card.applicability, Applicability::Mismatch { .. }) {
                    metrics.applicability_ok += 1;
                }
            }
            if let Some(g) = claim_to_gold.get(&card.id.to_string()) {
                ids.push(g.clone());
            }
        }
        score_id_list(&mut metrics, scenario, probe, &ids, limit);
    }
    Ok(metrics)
}

fn score_b0(metrics: &mut ErMetrics, probe: &Probe) {
    metrics.probes += 1;
    if probe.expect.abstain {
        metrics.abstain_total += 1;
        metrics.abstain_correct += 1;
    } else {
        metrics.expected_hits += probe.expect.claim_ids.len().max(1);
    }
}

fn score_id_list(
    metrics: &mut ErMetrics,
    scenario: &Scenario,
    probe: &Probe,
    ids: &[String],
    k: usize,
) {
    metrics.probes += 1;
    let top: BTreeSet<_> = ids.iter().take(k).cloned().collect();
    if probe.expect.abstain {
        metrics.abstain_total += 1;
        if top.is_empty() {
            metrics.abstain_correct += 1;
        }
        return;
    }
    let expected: BTreeSet<_> = probe.expect.claim_ids.iter().cloned().collect();
    metrics.expected_hits += expected.len().max(1);
    let hits = top.intersection(&expected).count();
    metrics.hits_at_k += hits;
    if !probe.include_history {
        for id in &top {
            if let Some(g) = scenario.gold.iter().find(|g| &g.id == id)
                && g.status != "active"
            {
                metrics.stale_hits += 1;
                metrics.ranked += 1;
            }
        }
    }
}
