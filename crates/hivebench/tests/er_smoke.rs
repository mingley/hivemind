use hivebench::baselines::{self, Condition};
use hivebench::run_er;
use hivebench::scenario::load_scenario;
use tempfile::TempDir;

#[test]
fn scenario_01_hivemind_hits_gold() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scenarios/01-recurring-ci-failure");
    let s = load_scenario(&root).expect("load");
    let tmp = TempDir::new().unwrap();
    let store = baselines::open_store(tmp.path()).unwrap();
    let map = baselines::ingest_given(&store, &s, true).unwrap();
    eprintln!("gold_map={map:?}");
    assert!(!map.is_empty(), "ingest should map gold ids");
    let probe = &s.probes[0];
    let q = baselines::probe_query(&probe.query, &probe.ctx, false, false, 8);
    let recall = store.recall(&q);
    eprintln!("query={:?} cards={}", probe.query, recall.cards.len());
    for c in &recall.cards {
        eprintln!(
            "  id={} stmt={}",
            c.id,
            &c.statement.chars().take(60).collect::<String>()
        );
    }
    let claim_to_gold: std::collections::BTreeMap<_, _> =
        map.into_iter().map(|(g, c)| (c, g)).collect();
    let mut hits = 0;
    for c in &recall.cards {
        if let Some(g) = claim_to_gold.get(&c.id.to_string()) {
            eprintln!("matched gold {g}");
            if probe.expect.claim_ids.contains(g) {
                hits += 1;
            }
        } else {
            eprintln!("no map for {}", c.id);
        }
    }
    eprintln!("direct hits={hits}");
    let m = run_er(&s, Condition::Hivemind, 8).unwrap();
    eprintln!(
        "metrics hits={} expected={} abstain_total={}",
        m.hits_at_k, m.expected_hits, m.abstain_total
    );
    assert!(m.hits_at_k > 0, "expected at least one hit, got {m:?}");
}
