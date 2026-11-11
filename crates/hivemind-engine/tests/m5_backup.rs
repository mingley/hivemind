//! M5: ship log segments for backup.

use hivemind_core::{Actor, ClaimData, ClaimKind};
use hivemind_engine::{Durability, Options, Remember, Store};
use tempfile::tempdir;

#[test]
fn backup_copies_segments_and_manifest() {
    let src = tempdir().unwrap();
    let dest = tempdir().unwrap();
    let store = Store::open(
        src.path(),
        Options {
            durability: Durability::NoFsync,
            ..Options::default()
        },
    )
    .unwrap();
    store
        .remember(
            &Actor::agent("a"),
            Remember::new(ClaimData::new(
                ClaimKind::Fact,
                "backup me",
                "project:acme/api".parse().unwrap(),
            )),
        )
        .unwrap();
    let report = store.backup_segments(dest.path()).unwrap();
    assert!(report.segments >= 1);
    assert!(dest.path().join("MANIFEST").exists());
    assert!(dest.path().join("log").exists());
}
