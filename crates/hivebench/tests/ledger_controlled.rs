//! Controlled-loop token ledger: measured schemas/cards, estimated embeddings.

use hivebench::baselines::Condition;
use hivebench::harness::{LoopOptions, run_controlled};
use hivebench::ledger::{Category, CountSource};
use hivebench::scenario::load_scenario;
use hivebench::tools::{hivemind_mcp_tools, tool_schema_tokens};

fn scenario_01() -> hivebench::Scenario {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scenarios/01-recurring-ci-failure");
    load_scenario(&root).expect("load scenario 01")
}

#[test]
fn controlled_hivemind_records_measured_and_estimated_categories() {
    let s = scenario_01();
    let run = run_controlled(
        &s,
        Condition::Hivemind,
        LoopOptions {
            run_id: "test".into(),
            expand_top: true,
            k: 8,
        },
    )
    .expect("controlled loop");

    assert_eq!(run.turns, s.probes.len());
    assert!(
        run.er.hits_at_k > 0,
        "expected retrieval hits: {:?}",
        run.er
    );

    let by = run.ledger.tokens_by_category();
    assert!(
        by.get("tool_schema").copied().unwrap_or(0) > 0,
        "tool_schema missing: {by:?}"
    );
    assert!(
        by.get("retrieved_context").copied().unwrap_or(0) > 0,
        "retrieved_context missing: {by:?}"
    );
    assert!(
        by.get("query_formulation").copied().unwrap_or(0) > 0,
        "query_formulation missing: {by:?}"
    );
    assert!(
        by.get("query_embedding").copied().unwrap_or(0) > 0,
        "query_embedding missing: {by:?}"
    );
    assert!(
        by.get("write_embedding").copied().unwrap_or(0) > 0,
        "write_embedding missing: {by:?}"
    );
    assert!(
        by.get("followup_expansion").copied().unwrap_or(0) > 0,
        "followup_expansion missing: {by:?}"
    );

    let schema_turn = tool_schema_tokens(Condition::Hivemind);
    let schema_entries: Vec<_> = run
        .ledger
        .entries
        .iter()
        .filter(|e| e.category == Category::ToolSchema)
        .collect();
    assert_eq!(schema_entries.len(), s.probes.len());
    for e in &schema_entries {
        assert_eq!(e.source, CountSource::Measured);
        assert_eq!(e.input_tokens, schema_turn);
    }

    let measured_cats = [
        Category::ToolSchema,
        Category::RetrievedContext,
        Category::QueryFormulation,
        Category::FollowupExpansion,
    ];
    for cat in measured_cats {
        assert!(
            run.ledger
                .entries
                .iter()
                .filter(|e| e.category == cat)
                .all(|e| e.source == CountSource::Measured),
            "{cat:?} should be measured"
        );
    }
    for cat in [Category::QueryEmbedding, Category::WriteEmbedding] {
        assert!(
            run.ledger
                .entries
                .iter()
                .filter(|e| e.category == cat)
                .all(|e| e.source == CountSource::Estimated),
            "{cat:?} should be estimated"
        );
    }

    assert!(run.ledger.memory_attributable_tokens() > 0);
    assert!(run.ledger.measured_tokens() > run.ledger.estimated_tokens());
    assert_eq!(hivemind_mcp_tools().len(), 8);
}

#[test]
fn controlled_b0_has_zero_memory_tool_overhead() {
    let s = scenario_01();
    let run = run_controlled(
        &s,
        Condition::B0,
        LoopOptions {
            run_id: "test".into(),
            expand_top: false,
            k: 8,
        },
    )
    .expect("b0 loop");

    let by = run.ledger.tokens_by_category();
    assert_eq!(by.get("tool_schema").copied().unwrap_or(0), 0);
    assert_eq!(by.get("retrieved_context").copied().unwrap_or(0), 0);
    assert_eq!(by.get("write_embedding").copied().unwrap_or(0), 0);
    assert_eq!(by.get("query_embedding").copied().unwrap_or(0), 0);
    assert_eq!(by.get("query_formulation").copied().unwrap_or(0), 0);
    assert_eq!(run.ledger.memory_attributable_tokens(), 0);
    assert!(run.ledger.entries.is_empty());
}

#[test]
fn dry_run_alias_matches_controlled_defaults() {
    let s = scenario_01();
    let a = hivebench::dry_run(&s, Condition::Hivemind, 8).unwrap();
    let b = run_controlled(&s, Condition::Hivemind, LoopOptions::default()).unwrap();
    assert_eq!(
        a.ledger.total_non_judge_tokens(),
        b.ledger.total_non_judge_tokens()
    );
    assert_eq!(a.ledger.entries.len(), b.ledger.entries.len());
}

#[test]
fn no_expand_skips_followup() {
    let s = scenario_01();
    let run = run_controlled(
        &s,
        Condition::Hivemind,
        LoopOptions {
            run_id: "test".into(),
            expand_top: false,
            k: 8,
        },
    )
    .unwrap();
    assert!(
        !run.ledger
            .entries
            .iter()
            .any(|e| e.category == Category::FollowupExpansion)
    );
}
