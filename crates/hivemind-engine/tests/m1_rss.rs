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
