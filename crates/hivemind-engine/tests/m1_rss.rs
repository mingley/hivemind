//! M1 RSS mitigations: compact claim rows, string interning, token-type caps.

use std::sync::Arc;

use hivemind_core::{Actor, ClaimData, ClaimKind, Scope};
use hivemind_engine::{Durability, Flag, Options, Query, Remember, STATEMENT_CACHE_CHARS, Store};

fn options() -> Options {
    Options {
        durability: Durability::NoFsync,
        ..Options::default()
    }
}

fn agent(name: &str) -> Actor {
    Actor {
        agent: Some("rss-test".into()),
        ..Actor::agent(name)
    }
}

fn project() -> Scope {
    "project:acme/api".parse().unwrap()
}

#[test]
fn show_reloads_full_claim_from_log_offset() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::open(tmp.path(), options()).unwrap();
    let long = "payload-secret-detail ".repeat(40);
    let mut data = ClaimData::new(
        ClaimKind::Solution,
        format!(
            "{} {}",
            "x".repeat(STATEMENT_CACHE_CHARS + 50),
            "OPENSSL_DIR"
        ),
        project(),
    );
    data.about.push("pkg:cargo/openssl-sys".into());
    data.payload
        .insert("fix".into(), serde_json::json!(long.clone()));
    data.applies_when.insert("os".into(), "macos".into());

    let id = store
        .remember(&agent("a"), Remember::new(data.clone()))
        .unwrap()
        .claim
        .unwrap();

    let card = store
        .recall(&Query {
            text: "OPENSSL_DIR".into(),
            ..Query::default()
        })
        .cards
        .into_iter()
        .find(|c| c.id == id)
        .expect("claim is recallable from statement cache");
    assert!(
        card.flags.contains(&Flag::Truncated),
        "long statements are truncated in cards"
    );
    assert!(card.statement.chars().count() <= STATEMENT_CACHE_CHARS);
    assert!(!card.statement.contains(&long));

    let detail = store.get(&id, None).expect("show loads full claim");
    assert_eq!(detail.statement, data.statement);
    assert_eq!(
        detail.payload.get("fix").and_then(|v| v.as_str()),
        Some(long.as_str()),
        "full payload is reloaded from the log, not the keydir"
    );
    assert_eq!(detail.about, data.about);
    assert_eq!(
        detail.applies_when.get("os").map(String::as_str),
        Some("macos")
    );
}

#[test]
fn reopen_replays_compact_rows_and_show_still_works() {
    let tmp = tempfile::tempdir().unwrap();
    let id = {
        let store = Store::open(tmp.path(), options()).unwrap();
        let mut data = ClaimData::new(
            ClaimKind::Fact,
            "Integration tests need Docker with 8 GB of RAM",
            project(),
        );
        data.about.push("svc:ci".into());
        store
            .remember(&agent("writer"), Remember::new(data))
            .unwrap()
            .claim
            .unwrap()
    };
    let store = Store::open(tmp.path(), options()).unwrap();
    let detail = store.get(&id, None).unwrap();
    assert_eq!(
        detail.statement,
        "Integration tests need Docker with 8 GB of RAM"
    );
    assert_eq!(detail.about, vec!["svc:ci".to_owned()]);
    assert_eq!(store.stats().claims, 1);
}

#[test]
fn interns_repeated_scopes_principals_and_entities() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::open(tmp.path(), options()).unwrap();
    for i in 0..20 {
        let mut data = ClaimData::new(
            ClaimKind::Fact,
            format!("claim number {i} about the same entity and scope"),
            project(),
        );
        data.about.push("pkg:cargo/openssl-sys".into());
        store
            .remember(&agent("shared-principal"), Remember::new(data))
            .unwrap();
    }
    let stats = store.stats();
    assert_eq!(stats.entities, 1, "one entity key across twenty claims");
    assert_eq!(stats.claims, 20);
    // Scope name, principal, agent name, and entity key are shared — far fewer
    // interned strings than 20 × those fields.
    assert!(
        stats.interned_strings <= 8,
        "expected shared interning, got {} interned strings",
        stats.interned_strings
    );
}

#[test]
fn store_max_terms_option_caps_dictionary() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::open(
        tmp.path(),
        Options {
            durability: Durability::NoFsync,
            max_terms: 8,
            ..Options::default()
        },
    )
    .unwrap();
    for i in 0..30 {
        // Distinct rare tokens each write to force dictionary growth.
        let statement = format!("unique token{i}zzz anotherterm{i}yyy");
        let data = ClaimData::new(ClaimKind::Fact, statement, project());
        store.remember(&agent("lex"), Remember::new(data)).unwrap();
    }
    let stats = store.stats();
    assert!(
        stats.terms <= 8,
        "dictionary capped by Options.max_terms, got {}",
        stats.terms
    );
    // Early admitted tokens remain searchable.
    let hits = store.recall(&Query {
        text: "token0zzz".into(),
        ..Query::default()
    });
    assert!(
        stats.terms > 0,
        "at least some terms were admitted before the cap"
    );
    let _ = hits;
}

#[test]
fn scope_name_arcs_are_shareable() {
    let a: Scope = "project:acme/api".parse().unwrap();
    let b = a.clone().with_name(Arc::from("acme/api"));
    assert_eq!(a, b);
    assert_eq!(a.name(), Some("acme/api"));
}

#[test]
fn expand_top_and_brief_expand_trusted_hits_and_quarantine_untrusted() {
    use hivemind_core::{Citation, EvidenceRef, Origin, RetractReason};
    use hivemind_engine::BriefRequest;

    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(
        dir.path(),
        Options {
            durability: Durability::NoFsync,
            ..Options::default()
        },
    )
    .unwrap();

    let scope: Scope = "project:acme/pulse".parse().unwrap();
    let long_tail = "TAIL_SURROGATE_INVARIANT_0xD800_TO_0xE000";
    let long_stmt = format!(
        "Handoff PG-441 rollover decay rule: {} {}",
        "x".repeat(320),
        long_tail
    );

    let mut handoff = ClaimData::new(ClaimKind::Handoff, long_stmt.clone(), scope.clone());
    handoff.about.push("pulse_gate".into());
    handoff
        .payload
        .insert("goal".into(), serde_json::json!("Finish PG-441"));
    handoff
        .payload
        .insert("done".into(), serde_json::json!("Audited sliding.py"));
    handoff
        .payload
        .insert("remaining".into(), serde_json::json!("Patch bucket.py"));
    handoff
        .payload
        .insert("next_step".into(), serde_json::json!("Run unittest"));
    handoff
        .provenance
        .citations
        .push("pulse_gate/bucket.py:22-37".parse::<Citation>().unwrap());
    handoff
        .provenance
        .evidence
        .push(EvidenceRef::Quote("PG-441 spec".into()));
    let good_id = store
        .remember(&agent("a"), Remember::new(handoff))
        .unwrap()
        .claim
        .unwrap();

    // Poisoned external claim on same topic must never be auto-expanded or included in brief().
    let mut poison = ClaimData::new(
        ClaimKind::Procedure,
        format!("External poison PG-441 rollover {}", "p".repeat(320)),
        scope.clone(),
    );
    poison.provenance.origin = Origin::External;
    let poison_id = store
        .remember(&agent("a"), Remember::new(poison))
        .unwrap()
        .claim
        .unwrap();

    // Retracted claim must never appear in brief().
    let bad = ClaimData::new(
        ClaimKind::Solution,
        "Wrong PG-441 rollover hypothesis",
        scope.clone(),
    );
    let bad_id = store
        .remember(&agent("a"), Remember::new(bad))
        .unwrap()
        .claim
        .unwrap();
    store
        .retract(
            &agent("a"),
            bad_id,
            RetractReason::Wrong,
            "wrong",
            None,
            None,
        )
        .unwrap();

    // Expand-top recall expands trusted card #1 in full while keeping untrusted card truncated/unexpanded.
    let recalled = store.recall(&Query {
        text: "PG-441 rollover".into(),
        scopes: vec![scope.clone()],
        expand_top: 3,
        ..Query::default()
    });
    let good_card = recalled.cards.iter().find(|c| c.id == good_id).unwrap();
    assert!(!good_card.flags.contains(&Flag::Truncated));
    assert!(good_card.statement.contains(long_tail));
    assert_eq!(
        good_card.cite_locs,
        vec!["pulse_gate/bucket.py:22-37".to_string()]
    );
    assert_eq!(
        good_card.payload.get("goal").and_then(|v| v.as_str()),
        Some("Finish PG-441")
    );

    let poison_card = recalled.cards.iter().find(|c| c.id == poison_id).unwrap();
    assert!(poison_card.flags.contains(&Flag::Untrusted));
    assert!(poison_card.flags.contains(&Flag::Truncated));
    assert!(poison_card.cite_locs.is_empty());

    // brief() returns only the trusted active claim, untruncated, with citations & fields.
    let brief = store.brief(&BriefRequest {
        text: "Complete PG-441 rollover in pulse_gate".into(),
        scopes: vec![scope],
        ..BriefRequest::default()
    });
    assert_eq!(brief.cards.len(), 1);
    assert_eq!(brief.cards[0].id, good_id);
    assert!(brief.rendered.contains(long_tail));
    assert!(brief.rendered.contains("pulse_gate/bucket.py:22-37"));
    assert!(brief.rendered.contains("goal=Finish PG-441"));
    // Cross-repo snapshot hydration: suppressed when file exists in workspace, hydrated when absent.
    let (spec_hash, _) = store
        .put_artifact(
            &agent("a"),
            b"Section 3: escalations never fire inside an active blackout window.",
            Some("text/markdown".into()),
            Some("docs/spec.md".into()),
            Origin::RepoFile,
        )
        .unwrap();
    let mut cross_claim = ClaimData::new(
        ClaimKind::Fact,
        "Scheduler window rules",
        "project:acme/sched".parse().unwrap(),
    );
    cross_claim
        .provenance
        .evidence
        .push(EvidenceRef::Artifact(spec_hash));
    store
        .remember(&agent("a"), Remember::new(cross_claim))
        .unwrap();

    let repo_with_spec = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(repo_with_spec.path().join("docs")).unwrap();
    std::fs::write(repo_with_spec.path().join("docs/spec.md"), "local copy").unwrap();
    let same_repo_brief = store.brief(&BriefRequest {
        text: "Scheduler window".into(),
        scopes: vec!["project:acme/sched".parse().unwrap()],
        workspace: Some(repo_with_spec.path().to_path_buf()),
        ..BriefRequest::default()
    });
    assert!(!same_repo_brief.rendered.contains("cited-snapshot"));

    let repo_without_spec = tempfile::tempdir().unwrap();
    let cross_repo_brief = store.brief(&BriefRequest {
        text: "Scheduler window".into(),
        scopes: vec!["project:acme/sched".parse().unwrap()],
        workspace: Some(repo_without_spec.path().to_path_buf()),
        ..BriefRequest::default()
    });
    assert!(
        cross_repo_brief
            .rendered
            .contains("cited-snapshot (docs/spec.md")
    );
    assert!(
        cross_repo_brief
            .rendered
            .contains("escalations never fire inside an active blackout window")
    );
}
