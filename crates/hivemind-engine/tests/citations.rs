//! M3: read-time citation verification against a workspace.

use std::fs;

use hivemind_core::{
    Actor, Citation, CitationStatus, ClaimData, ClaimKind, Method, Origin, Provenance,
    hash_file_citation,
};
use hivemind_engine::{Durability, Options, Remember, Store};
use tempfile::tempdir;

fn options() -> Options {
    Options {
        durability: Durability::NoFsync,
        ..Options::default()
    }
}

#[test]
fn show_reports_valid_then_drifted_citations() {
    let store_dir = tempdir().unwrap();
    let workspace = tempdir().unwrap();
    let file = workspace.path().join("src/fix.rs");
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, "line1\nOPENSSL_DIR must be set\nline3\n").unwrap();

    let store = Store::open(store_dir.path(), options()).unwrap();
    let actor = Actor::agent("tester");
    let mut cite = Citation {
        path: "src/fix.rs".into(),
        start_line: Some(2),
        end_line: Some(2),
        ..Citation::default()
    };
    cite.content_hash = Some(hash_file_citation(workspace.path(), &cite).unwrap());

    let mut data = ClaimData::new(
        ClaimKind::Solution,
        "Set OPENSSL_DIR on macOS arm64",
        "project:acme/api".parse().unwrap(),
    );
    data.provenance = Provenance {
        method: Method::AgentAuthored,
        origin: Origin::Agent,
        evidence: vec![],
        citations: vec![cite],
        derived_from: vec![],
    };
    let receipt = store
        .remember(
            &actor,
            Remember {
                data,
                supersedes: vec![],
                idempotency_key: None,
            },
        )
        .unwrap();
    let id = receipt.claim.unwrap();

    let detail = store
        .get_with_workspace(&id, None, Some(workspace.path()))
        .unwrap();
    assert_eq!(detail.citation_checks.len(), 1);
    assert_eq!(detail.citation_checks[0].status, CitationStatus::Valid);

    fs::write(&file, "line1\nchanged\nline3\n").unwrap();
    let detail = store
        .get_with_workspace(&id, None, Some(workspace.path()))
        .unwrap();
    assert_eq!(detail.citation_checks[0].status, CitationStatus::Drifted);
}
