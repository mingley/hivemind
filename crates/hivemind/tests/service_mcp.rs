//! In-process Service dispatch sequence and MCP tool-router name order.

use hivemind::mcp::HiveMcp;
use hivemind::service::Service;
use hivemind_engine::{Durability, Options, Store};
use serde_json::json;

fn open_service() -> (tempfile::TempDir, Service) {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::open(
        tmp.path(),
        Options {
            durability: Durability::NoFsync,
            ..Options::default()
        },
    )
    .unwrap();
    (tmp, Service::new(store))
}

#[test]
fn mcp_tools_list_six_names_in_deterministic_order() {
    // rmcp's ToolRouter::list_all sorts by name; that sorted order is what
    // clients see and must stay stable for prompt-cache friendliness.
    let names: Vec<String> = HiveMcp::tool_router()
        .list_all()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();
    assert_eq!(
        names,
        vec![
            "changes_since",
            "feedback",
            "recall",
            "remember",
            "retract",
            "show",
        ]
    );
    // Second call must match (deterministic).
    let again: Vec<String> = HiveMcp::tool_router()
        .list_all()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();
    assert_eq!(names, again);
}

#[test]
fn service_dispatch_remember_recall_show_feedback_changes_since() {
    let (_tmp, service) = open_service();

    let remembered = service
        .dispatch(
            "remember",
            json!({
                "statement": "Service handlers cover the MCP write/read path in-process",
                "kind": "fact",
                "scope": "project:acme/service",
            }),
        )
        .expect("remember");
    let claim = remembered["claim"].as_str().unwrap().to_owned();
    let offset = remembered["offset"].as_u64().unwrap();

    let recalled = service
        .dispatch(
            "recall",
            json!({
                "query": "MCP write/read path",
                "min_offset": offset,
            }),
        )
        .expect("recall");
    assert!(
        recalled["cards"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["id"] == claim),
        "{recalled}"
    );

    let shown = service
        .dispatch(
            "show",
            json!({
                "id": claim,
                "min_offset": offset,
            }),
        )
        .expect("show");
    assert_eq!(shown["id"], claim);
    assert!(
        shown["statement"]
            .as_str()
            .unwrap()
            .contains("Service handlers")
    );

    let feedback = service
        .dispatch(
            "feedback",
            json!({
                "id": claim,
                "outcome": "helpful",
                "note": "covered by service_mcp test",
            }),
        )
        .expect("feedback");
    assert!(feedback["offset"].as_u64().unwrap() > offset);

    let changes = service
        .dispatch(
            "changes_since",
            json!({
                "from": 0,
                "limit": 50,
                "filter": {
                    "claims": [claim],
                },
            }),
        )
        .expect("changes_since");
    let events = changes["events"].as_array().unwrap();
    assert!(
        events.len() >= 2,
        "expect claim_asserted + feedback_recorded, got {changes}"
    );
    assert!(events.iter().any(|row| {
        row["event"]["body"]["type"] == "claim_asserted" && row["event"]["body"]["claim"] == claim
    }));
    assert!(events.iter().any(|row| {
        row["event"]["body"]["type"] == "feedback_recorded"
            && row["event"]["body"]["claim"] == claim
            && row["event"]["body"]["outcome"] == "helpful"
    }));
}
