//! M5: HTTP service with tenant partitions and bearer auth.

use std::net::SocketAddr;
use std::time::Duration;

use hivemind::http::{self, ServeArgs};
use serde_json::json;
use tempfile::tempdir;

#[tokio::test]
async fn health_is_open_and_rpc_requires_bearer() {
    let root = tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let token = "test-token".to_owned();
    let serve = tokio::spawn(http::serve(ServeArgs {
        root: root.path().to_path_buf(),
        listen: addr,
        token: token.clone(),
        no_fsync: true,
    }));

    // Wait until the server accepts connections.
    for _ in 0..50 {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let client = reqwest::Client::new();
    let health = client
        .get(format!("http://{addr}/health"))
        .send()
        .await
        .unwrap();
    assert_eq!(health.status(), 200);

    let denied = client
        .get(format!("http://{addr}/v1/tenants"))
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), 401);

    let tenants = client
        .get(format!("http://{addr}/v1/tenants"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(tenants.status(), 200);

    let remember = client
        .post(format!("http://{addr}/v1/tenants/acme/rpc"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "method": "remember",
            "params": {
                "statement": "hello from http",
                "kind": "fact",
                "scope": "project:acme/api",
                "actor": { "principal": "tester" }
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(remember.status(), 200, "{}", remember.text().await.unwrap());
    let body: serde_json::Value = remember.json().await.unwrap();
    assert!(body.get("offset").is_some() || body.get("claim").is_some());

    let changes = client
        .get(format!("http://{addr}/v1/tenants/acme/changes?from=0"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(changes.status(), 200);

    serve.abort();
}

#[allow(dead_code)]
fn _addr(_: SocketAddr) {}
