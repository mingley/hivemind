//! M6: projection checkpoints skip re-apply of snapshotted log offsets.

use hivemind_core::{Actor, ClaimData, ClaimKind, Scope};
use hivemind_engine::{Durability, Options, Query, Remember, Store};

fn options() -> Options {
    Options {
        durability: Durability::NoFsync,
        ..Options::default()
    }
}

fn agent() -> Actor {
    Actor::agent("m6")
}

#[test]
fn checkpoint_then_reopen_restores_claims_via_snapshot() {
    let tmp = tempfile::tempdir().unwrap();
    {
        let store = Store::open(tmp.path(), options()).unwrap();
        let data = ClaimData::new(
            ClaimKind::Fact,
            "snapshots speed reopen after many events",
            "project:acme/api".parse::<Scope>().unwrap(),
        );
        store.remember(&agent(), Remember::new(data)).unwrap();
        let info = store.checkpoint().unwrap();
        assert!(info.events >= 1);
        assert_eq!(info.next_offset, store.next_offset());
    }

    let store = Store::open(tmp.path(), options()).unwrap();
    assert!(store.snapshot_offset() > 0);
    let recall = store.recall(&Query {
        text: "snapshots reopen".into(),
        ..Query::default()
    });
    assert!(
        recall
            .cards
            .iter()
            .any(|c| c.statement.contains("snapshots speed reopen")),
        "{recall:?}"
    );
}

#[test]
fn checkpoint_then_new_writes_replay_only_tail() {
    let tmp = tempfile::tempdir().unwrap();
    {
        let store = Store::open(tmp.path(), options()).unwrap();
        for i in 0..5 {
            let data = ClaimData::new(
                ClaimKind::Fact,
                format!("seed claim number {i}"),
                "project:acme/api".parse::<Scope>().unwrap(),
            );
            store.remember(&agent(), Remember::new(data)).unwrap();
        }
        store.checkpoint().unwrap();
        let data = ClaimData::new(
            ClaimKind::Fact,
            "post-checkpoint unique phrase zebraquill",
            "project:acme/api".parse::<Scope>().unwrap(),
        );
        store.remember(&agent(), Remember::new(data)).unwrap();
    }

    let store = Store::open(tmp.path(), options()).unwrap();
    assert!(store.snapshot_offset() >= 5);
    let recall = store.recall(&Query {
        text: "zebraquill".into(),
        ..Query::default()
    });
    assert!(
        recall
            .cards
            .iter()
            .any(|c| c.statement.contains("zebraquill")),
        "{recall:?}"
    );
}
