//! M1 engine surfaces: [`Store::wait_for_offset`] and filtered [`Store::changes_since`].

use std::time::Duration;

use hivemind_core::{Actor, ClaimData, ClaimKind, Scope};
use hivemind_engine::{ChangesFilter, Durability, Error, Options, Remember, Store};

fn options() -> Options {
    Options {
        durability: Durability::NoFsync,
        ..Options::default()
    }
}

fn agent() -> Actor {
    Actor::agent("m1-feed-test")
}

fn scope(s: &str) -> Scope {
    s.parse().unwrap()
}

fn remember(
    store: &Store,
    kind: ClaimKind,
    statement: &str,
    scope: Scope,
) -> (u64, hivemind_core::ClaimId) {
    let receipt = store
        .remember(
            &agent(),
            Remember::new(ClaimData::new(kind, statement, scope)),
        )
        .unwrap();
    (receipt.offset, receipt.claim.unwrap())
}

#[test]
fn wait_for_offset_succeeds_once_projections_pass_min() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::open(tmp.path(), options()).unwrap();
    let (offset, _) = remember(
        &store,
        ClaimKind::Fact,
        "wait_for_offset sees committed writes",
        scope("project:acme/wait"),
    );
    store
        .wait_for_offset(offset, Duration::from_millis(200))
        .expect("already applied offset must succeed immediately");
    assert!(store.next_offset() > offset);
}

#[test]
fn wait_for_offset_times_out_when_cursor_is_ahead() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::open(tmp.path(), options()).unwrap();
    let ahead = store.next_offset();
    let err = store
        .wait_for_offset(ahead, Duration::from_millis(40))
        .expect_err("future offset with no writer must time out");
    assert!(
        matches!(err, Error::Timeout(_)),
        "expected Timeout, got {err:?}"
    );
}

#[test]
fn changes_since_filters_by_scope_and_claim() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::open(tmp.path(), options()).unwrap();

    let (_, api_id) = remember(
        &store,
        ClaimKind::Fact,
        "API uses bearer tokens in Integration tests",
        scope("project:acme/api"),
    );
    let (_, web_id) = remember(
        &store,
        ClaimKind::Decision,
        "Web frontend ships on Fridays",
        scope("project:acme/web"),
    );
    let _ = remember(
        &store,
        ClaimKind::Solution,
        "Set OPENSSL_DIR for arm64 linking",
        scope("project:acme/api"),
    );

    let from = 0u64;
    let all = store
        .changes_since(from, 100, &ChangesFilter::default())
        .unwrap();
    assert!(all.len() >= 3);

    let api_only = store
        .changes_since(
            from,
            100,
            &ChangesFilter {
                scopes: Some(vec![scope("project:acme/api")]),
                ..ChangesFilter::default()
            },
        )
        .unwrap();
    assert!(!api_only.is_empty());
    assert!(api_only.iter().all(|(_, ev)| {
        ev.body
            .claim()
            .and_then(|id| store.get(&id, None))
            .is_some_and(|d| d.scope == scope("project:acme/api"))
    }));
    assert!(
        api_only
            .iter()
            .any(|(_, ev)| ev.body.claim() == Some(api_id))
    );
    assert!(
        api_only
            .iter()
            .all(|(_, ev)| ev.body.claim() != Some(web_id))
    );

    let one_claim = store
        .changes_since(
            from,
            100,
            &ChangesFilter {
                claims: Some(vec![web_id]),
                ..ChangesFilter::default()
            },
        )
        .unwrap();
    assert_eq!(one_claim.len(), 1);
    assert_eq!(one_claim[0].1.body.claim(), Some(web_id));

    let decisions = store
        .changes_since(
            from,
            100,
            &ChangesFilter {
                kinds: Some(vec![ClaimKind::Decision]),
                ..ChangesFilter::default()
            },
        )
        .unwrap();
    assert_eq!(decisions.len(), 1);
    assert_eq!(decisions[0].1.body.claim(), Some(web_id));
}
