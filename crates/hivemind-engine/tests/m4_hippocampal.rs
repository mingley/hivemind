//! M4 hippocampal mechanisms: C5 context contrast, C6 supersede match.

use hivemind_core::{Actor, ClaimData, ClaimKind, EdgeKind};
use hivemind_engine::{Durability, Error, Flag, Options, Remember, Store};
use tempfile::tempdir;

fn options() -> Options {
    Options {
        durability: Durability::NoFsync,
        ..Options::default()
    }
}

#[test]
fn supersede_rejects_conflicting_applicability() {
    let dir = tempdir().unwrap();
    let store = Store::open(dir.path(), options()).unwrap();
    let actor = Actor::agent("tester");

    let mut mac = ClaimData::new(
        ClaimKind::Solution,
        "Set OPENSSL_DIR for the linker on this platform",
        "project:acme/api".parse().unwrap(),
    );
    mac.applies_when.insert("os".into(), "macos".into());
    let old = store
        .remember(&actor, Remember::new(mac))
        .unwrap()
        .claim
        .unwrap();

    let mut linux = ClaimData::new(
        ClaimKind::Solution,
        "Set OPENSSL_DIR for the linker on this platform",
        "project:acme/api".parse().unwrap(),
    );
    linux.applies_when.insert("os".into(), "linux".into());
    let err = store
        .remember(
            &actor,
            Remember {
                data: linux.clone(),
                supersedes: vec![old],
                idempotency_key: None,
            },
        )
        .unwrap_err();
    assert!(matches!(err, Error::Conflict(_)), "{err}");

    let sibling = store
        .remember(&actor, Remember::new(linux))
        .unwrap()
        .claim
        .unwrap();
    store
        .link(
            &actor,
            sibling,
            old,
            EdgeKind::ContrastsWith,
            Some("differs in os"),
            Vec::new(),
            None,
        )
        .unwrap();
}

#[test]
fn similar_near_duplicates_report_differs_in() {
    let dir = tempdir().unwrap();
    let store = Store::open(dir.path(), options()).unwrap();
    let actor = Actor::agent("tester");

    let mut mac = ClaimData::new(
        ClaimKind::Solution,
        "Homebrew openssl linker path fix for arm64 builds",
        "project:acme/api".parse().unwrap(),
    );
    mac.applies_when.insert("os".into(), "macos".into());
    store.remember(&actor, Remember::new(mac)).unwrap();

    let mut linux = ClaimData::new(
        ClaimKind::Solution,
        "Homebrew openssl linker path fix for arm64 builds",
        "project:acme/api".parse().unwrap(),
    );
    linux.applies_when.insert("os".into(), "linux".into());
    let receipt = store.remember(&actor, Remember::new(linux)).unwrap();
    assert!(
        receipt.similar.iter().any(
            |c| c.flags.contains(&Flag::ContextContrast) && c.differs_in.contains(&"os".into())
        ),
        "expected ContextContrast with differs_in=os, got {:?}",
        receipt.similar
    );
}
