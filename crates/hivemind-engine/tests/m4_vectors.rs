//! M4: embedding spaces, quantized scan, RRF fusion, hippocampal boosts.

use std::path::PathBuf;
use std::sync::Arc;

use hivemind_core::{Actor, ClaimData, ClaimKind};
use hivemind_engine::{
    Durability, EmbedderChoice, HashEmbedder, Options, Query, Remember, Store, build_embedder,
};
use tempfile::tempdir;

fn options() -> Options {
    Options {
        durability: Durability::NoFsync,
        ..Options::default()
    }
}

#[test]
fn lexical_default_remember_recall_without_embedder() {
    let dir = tempdir().unwrap();
    let store = Store::open(dir.path(), options()).unwrap();
    assert!(store.embedding_space().is_none());
    assert_eq!(store.vector_count(), 0);

    let actor = Actor::agent("tester");
    let data = ClaimData::new(
        ClaimKind::Solution,
        "OpenSSL linker miss on macOS arm64 needs OPENSSL_DIR",
        "project:acme/api".parse().unwrap(),
    );
    store.remember(&actor, Remember::new(data)).unwrap();
    assert_eq!(store.vector_count(), 0);

    let hits = store.recall(&Query {
        text: "OPENSSL_DIR macos linker".into(),
        limit: Some(3),
        ..Query::default()
    });
    assert_eq!(hits.cards.len(), 1);
    assert!(hits.cards[0].statement.contains("OPENSSL_DIR"));
}

#[test]
fn build_embedder_none_keeps_feature_off_default() {
    assert!(build_embedder(&EmbedderChoice::None).unwrap().is_none());
    let err = match build_embedder(&EmbedderChoice::Model2Vec {
        model: PathBuf::from("/nonexistent-model2vec"),
        revision: None,
    }) {
        Ok(_) => panic!("expected error"),
        Err(e) => e,
    };
    #[cfg(feature = "embeddings")]
    assert!(
        err.contains("model2vec") || err.contains("nonexistent"),
        "unexpected err: {err}"
    );
    #[cfg(not(feature = "embeddings"))]
    assert!(
        err.contains("--features embeddings"),
        "unexpected err: {err}"
    );
}

#[test]
fn hybrid_recall_fuses_lexical_and_vectors() {
    let dir = tempdir().unwrap();
    let store = Store::open(dir.path(), options()).unwrap();
    let actor = Actor::agent("tester");
    store.set_embedder(Arc::new(HashEmbedder::new(64))).unwrap();

    let mut a = ClaimData::new(
        ClaimKind::Solution,
        "OpenSSL linker miss on macOS arm64 needs OPENSSL_DIR",
        "project:acme/api".parse().unwrap(),
    );
    a.about.push("pkg:cargo/openssl-sys".into());
    store.remember(&actor, Remember::new(a)).unwrap();

    let b = ClaimData::new(
        ClaimKind::Handoff,
        "goal finished rate limiter skeleton",
        "project:acme/api".parse().unwrap(),
    );
    // handoff needs payload
    let mut b = b;
    b.payload
        .insert("goal".into(), serde_json::json!("limiter"));
    b.payload
        .insert("done".into(), serde_json::json!(["skeleton"]));
    b.payload
        .insert("remaining".into(), serde_json::json!(["tests"]));
    b.payload
        .insert("next_step".into(), serde_json::json!("write tests"));
    store.remember(&actor, Remember::new(b)).unwrap();

    assert_eq!(store.vector_count(), 2);
    let hits = store.recall(&Query {
        text: "openssl macos linker OPENSSL_DIR".into(),
        entity_boost: true,
        limit: Some(2),
        ..Query::default()
    });
    assert!(!hits.cards.is_empty());
    assert!(
        hits.cards[0]
            .statement
            .to_ascii_lowercase()
            .contains("openssl"),
        "hybrid recall should prefer the openssl claim"
    );
}

#[test]
fn int8_space_indexes_and_searches() {
    let dir = tempdir().unwrap();
    let store = Store::open(dir.path(), options()).unwrap();
    let actor = Actor::agent("tester");
    store
        .set_embedder(Arc::new(HashEmbedder::int8(48)))
        .unwrap();
    let data = ClaimData::new(
        ClaimKind::Fact,
        "shared fixture race causes flaky sync test",
        "project:acme/api".parse().unwrap(),
    );
    store.remember(&actor, Remember::new(data)).unwrap();
    let hits = store.recall(&Query {
        text: "flaky race shared fixture".into(),
        limit: Some(1),
        ..Query::default()
    });
    assert_eq!(hits.cards.len(), 1);
}

#[test]
fn abstention_min_score_can_drop_weak_hits() {
    let dir = tempdir().unwrap();
    let store = Store::open(dir.path(), options()).unwrap();
    let actor = Actor::agent("tester");
    let data = ClaimData::new(
        ClaimKind::Fact,
        "totally unrelated gardening tip",
        "project:acme/api".parse().unwrap(),
    );
    store.remember(&actor, Remember::new(data)).unwrap();
    let hits = store.recall(&Query {
        text: "openssl ssl_ctx_new linker".into(),
        min_score: Some(10_000.0),
        limit: Some(5),
        ..Query::default()
    });
    assert!(hits.cards.is_empty());
}
