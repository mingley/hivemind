//! M3: leases with fencing tokens.

use std::time::Duration;

use hivemind_core::{Actor, ClaimData, ClaimKind};
use hivemind_engine::{Durability, Error, Options, Remember, Store};
use tempfile::tempdir;

fn options() -> Options {
    Options {
        durability: Durability::NoFsync,
        ..Options::default()
    }
}

#[test]
fn fencing_rejects_stale_token_and_foreign_holder() {
    let dir = tempdir().unwrap();
    let store = Store::open(dir.path(), options()).unwrap();
    let a = Actor::agent("agent-a");
    let b = Actor::agent("agent-b");

    let first = store
        .acquire_lease(&a, "wi:parser", Duration::from_secs(3600), Some("parsing"))
        .unwrap();
    assert_eq!(first.lease.token, 1);
    assert!(first.lease.active);

    let conflict = store.acquire_lease(&b, "wi:parser", Duration::from_secs(60), None);
    assert!(matches!(conflict, Err(Error::Conflict(_))));

    let stale = store.release_lease(&a, "wi:parser", 999);
    assert!(matches!(stale, Err(Error::Conflict(_))));

    store
        .renew_lease(
            &a,
            "wi:parser",
            first.lease.token,
            Duration::from_secs(7200),
        )
        .unwrap();
    store
        .release_lease(&a, "wi:parser", first.lease.token)
        .unwrap();
    assert!(!store.lease("wi:parser").unwrap().active);

    let second = store
        .acquire_lease(&b, "wi:parser", Duration::from_secs(60), None)
        .unwrap();
    assert_eq!(second.lease.token, 2);
    assert_eq!(second.lease.holder, "agent-b");
}

#[test]
fn remember_sets_default_review_by_for_solutions() {
    let dir = tempdir().unwrap();
    let store = Store::open(dir.path(), options()).unwrap();
    let actor = Actor::agent("tester");
    let data = ClaimData::new(
        ClaimKind::Solution,
        "pin the transitive dep",
        "project:acme/api".parse().unwrap(),
    );
    assert!(data.review_by.is_none());
    let receipt = store.remember(&actor, Remember::new(data)).unwrap();
    let detail = store.get(&receipt.claim.unwrap(), None).unwrap();
    assert!(detail.review_by.is_some());
}
