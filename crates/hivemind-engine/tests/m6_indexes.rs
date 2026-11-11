//! M6: seal lexical mmap segments and build ANN from the store API.

use std::sync::Arc;

use hivemind_core::{Actor, ClaimData, ClaimKind};
use hivemind_engine::{Durability, HashEmbedder, LexicalSegment, Options, Query, Remember, Store};
use tempfile::tempdir;

fn options() -> Options {
    Options {
        durability: Durability::NoFsync,
        ..Options::default()
    }
}

#[test]
fn seal_lexical_round_trips_search() {
    let dir = tempdir().unwrap();
    let store = Store::open(dir.path(), options()).unwrap();
    let actor = Actor::agent("m6");
    store
        .remember(
            &actor,
            Remember::new(ClaimData::new(
                ClaimKind::Fact,
                "mmap lexical segment seals cleanly",
                "project:acme/api".parse().unwrap(),
            )),
        )
        .unwrap();
    let path = store.seal_lexical().unwrap();
    let seg = LexicalSegment::open(&path).unwrap();
    let hits = seg.search("mmap lexical");
    assert!(!hits.is_empty());
}

#[test]
fn build_ann_used_on_hybrid_recall() {
    let dir = tempdir().unwrap();
    let store = Store::open(dir.path(), options()).unwrap();
    let actor = Actor::agent("m6");
    store.set_embedder(Arc::new(HashEmbedder::new(32))).unwrap();
    for i in 0..80 {
        store
            .remember(
                &actor,
                Remember::new(ClaimData::new(
                    ClaimKind::Fact,
                    format!("vector scope claim {i} about openssl linker"),
                    "project:acme/api".parse().unwrap(),
                )),
            )
            .unwrap();
    }
    store.build_ann().unwrap();
    let hits = store.recall(&Query {
        text: "openssl linker".into(),
        limit: Some(5),
        ..Query::default()
    });
    assert!(!hits.cards.is_empty());
}
