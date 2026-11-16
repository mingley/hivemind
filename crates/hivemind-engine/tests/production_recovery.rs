//! Full restore and interrupted privacy-erasure contracts.
use hivemind_core::{Actor, ClaimData, ClaimKind, EvidenceRef, Origin};
use hivemind_engine::{Options, Remember, Store};

fn seed(store: &Store) -> (hivemind_core::ClaimId, hivemind_core::BlobHash) {
    let actor = Actor::agent("recovery-test");
    let (hash, _) = store
        .put_artifact(&actor, b"supporting transcript", None, None, Origin::Agent)
        .unwrap();
    let mut data = ClaimData::new(
        ClaimKind::Fact,
        "customer private text with supporting evidence",
        "project:restore".parse().unwrap(),
    );
    data.provenance.evidence.push(EvidenceRef::Artifact(hash));
    let id = store
        .remember(&actor, Remember::new(data))
        .unwrap()
        .claim
        .unwrap();
    (id, hash)
}

#[test]
fn full_backup_restores_evidence_and_rejects_overwrite_and_nested_destination() {
    let src = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    let store = Store::open(src.path(), Options::default()).unwrap();
    let (id, hash) = seed(&store);
    store.checkpoint().unwrap();
    let report = store.backup_segments(dest.path()).unwrap();
    assert_eq!(report.blobs, 1);
    let restored = Store::open(dest.path(), Options::default()).unwrap();
    assert_eq!(
        restored.get(&id, None).unwrap().statement,
        "customer private text with supporting evidence"
    );
    assert_eq!(restored.artifact(&hash).unwrap(), b"supporting transcript");
    assert!(restored.verify().unwrap().is_ok());
    assert!(store.backup_segments(dest.path()).is_err());
    assert!(store.backup_segments(&src.path().join("nested")).is_err());
    assert!(store.verify().unwrap().is_ok());
}

#[test]
fn missing_blob_refuses_to_publish_backup() {
    let src = tempfile::tempdir().unwrap();
    let parent = tempfile::tempdir().unwrap();
    let dest = parent.path().join("backup");
    let store = Store::open(src.path(), Options::default()).unwrap();
    let (_, hash) = seed(&store);
    let hex = hash.to_hex();
    std::fs::remove_file(src.path().join("blobs").join(&hex[..2]).join(&hex[2..])).unwrap();
    assert!(store.backup_segments(&dest).is_err());
    assert!(!dest.exists(), "failed backups must not appear complete");
    assert_eq!(std::fs::read_dir(parent.path()).unwrap().count(), 0);
}

#[test]
fn failed_blob_delete_is_reported_and_retried_after_restart() {
    let src = tempfile::tempdir().unwrap();
    let id = {
        let store = Store::open(src.path(), Options::default()).unwrap();
        let (id, hash) = seed(&store);
        store.checkpoint().unwrap();
        let hex = hash.to_hex();
        let blob = src.path().join("blobs").join(&hex[..2]).join(&hex[2..]);
        std::fs::remove_file(&blob).unwrap();
        std::fs::create_dir(&blob).unwrap(); // remove_file must fail, even as root
        assert!(
            store
                .forget(&Actor::agent("privacy"), id, "privacy erasure", None)
                .is_err()
        );
        assert!(store.get(&id, None).is_none());
        std::fs::remove_dir(&blob).unwrap();
        id
    };
    let store = Store::open(src.path(), Options::default()).unwrap();
    assert!(store.get(&id, None).is_none());
    assert_eq!(
        std::fs::read_dir(src.path().join("snapshots"))
            .unwrap()
            .count(),
        0
    );
    // The active writer must point at the new segment inode after redaction.
    let later = store
        .remember(
            &Actor::agent("writer"),
            Remember::new(ClaimData::new(
                ClaimKind::Fact,
                "written after erasure",
                "project:restore".parse().unwrap(),
            )),
        )
        .unwrap()
        .claim
        .unwrap();
    drop(store);
    let reopened = Store::open(src.path(), Options::default()).unwrap();
    assert!(reopened.get(&later, None).is_some());
    assert!(reopened.verify().unwrap().is_ok());
}

#[test]
fn snapshot_cleanup_failure_prevents_erasure_acknowledgement() {
    let src = tempfile::tempdir().unwrap();
    let store = Store::open(src.path(), Options::default()).unwrap();
    let (id, _) = seed(&store);
    let snapshot = store.checkpoint().unwrap().path;
    std::fs::remove_file(&snapshot).unwrap();
    std::fs::create_dir(&snapshot).unwrap();
    assert!(
        store
            .forget(&Actor::agent("privacy"), id, "privacy erasure", None)
            .is_err()
    );
    std::fs::remove_dir(&snapshot).unwrap();
    store
        .forget(&Actor::agent("privacy"), id, "retry", None)
        .unwrap();
    assert!(store.verify().unwrap().is_ok());
}

#[test]
fn capacity_refuses_new_claims_but_preserves_reads_and_idempotent_retries() {
    let src = tempfile::tempdir().unwrap();
    let store = Store::open(
        src.path(),
        Options {
            max_claims: 1,
            ..Options::default()
        },
    )
    .unwrap();
    let actor = Actor::agent("capacity");
    let mut first = Remember::new(ClaimData::new(
        ClaimKind::Fact,
        "capacity test",
        "global".parse().unwrap(),
    ));
    first.idempotency_key = Some("first".into());
    let receipt = store.remember(&actor, first.clone()).unwrap();
    assert!(store.remember(&actor, first).unwrap().replayed);
    assert!(
        store
            .remember(
                &actor,
                Remember::new(ClaimData::new(
                    ClaimKind::Fact,
                    "over capacity",
                    "global".parse().unwrap()
                ))
            )
            .is_err()
    );
    assert!(store.get(&receipt.claim.unwrap(), None).is_some());
    assert!(store.verify().unwrap().is_ok());
}
