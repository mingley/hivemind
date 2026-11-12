//! M5: gRPC Hello/Rpc with bearer auth.

use std::time::Duration;

use std::sync::Arc;

use hivemind::grpc::{self, GrpcArgs};
use hivemind::oauth::{
    DEFAULT_ACCESS_TOKEN_TTL_SECS, DEFAULT_REFRESH_TOKEN_TTL_SECS, OAuthConfig, OAuthState,
};
use hivemind::ratelimit::{RateLimitConfig, RateLimiter};
use tempfile::tempdir;
use tonic::Request;
use tonic::metadata::MetadataValue;

#[tokio::test]
async fn grpc_hello_and_rpc_with_bearer() {
    let root = tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let token = "grpc-bootstrap".to_owned();
    let issuer = format!("http://{addr}");
    let oauth = OAuthState::new(OAuthConfig {
        issuer: issuer.clone(),
        client_id: "hivemind-cli".into(),
        client_secret: "grpc-client-secret".into(),
        signing_key: "grpc-signing-key".into(),
        resource: format!("{issuer}/mcp"),
        access_token_ttl_secs: DEFAULT_ACCESS_TOKEN_TTL_SECS,
        refresh_token_ttl_secs: DEFAULT_REFRESH_TOKEN_TTL_SECS,
    });

    let serve = tokio::spawn(grpc::serve(GrpcArgs {
        root: root.path().to_path_buf(),
        listen: addr,
        token: token.clone(),
        oauth,
        limiter: Arc::new(RateLimiter::new(RateLimitConfig::default())),
        no_fsync: true,
    }));

    for _ in 0..50 {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let mut client = grpc::pb::hivemind_client::HivemindClient::connect(format!("http://{addr}"))
        .await
        .unwrap();

    let mut hello = Request::new(grpc::pb::HelloRequest {
        tenant: "acme".into(),
    });
    hello.metadata_mut().insert(
        "authorization",
        MetadataValue::try_from(format!("Bearer {token}")).unwrap(),
    );
    let resp = client.hello(hello).await.unwrap().into_inner();
    assert!(resp.json.contains("acme") || resp.json.contains("ok") || !resp.json.is_empty());

    let mut rpc = Request::new(grpc::pb::RpcRequest {
        tenant: "acme".into(),
        method: "remember".into(),
        params_json: serde_json::json!({
            "statement": "from grpc",
            "kind": "fact",
            "scope": "project:acme/api",
            "actor": { "principal": "spoofed" }
        })
        .to_string(),
    });
    rpc.metadata_mut().insert(
        "authorization",
        MetadataValue::try_from(format!("Bearer {token}")).unwrap(),
    );
    let remembered = client.rpc(rpc).await.unwrap().into_inner();
    assert!(
        remembered.json.contains("offset") || remembered.json.contains("claim"),
        "{}",
        remembered.json
    );

    serve.abort();
}

#[tokio::test]
async fn grpc_rpc_rate_limit_resource_exhausted() {
    let root = tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let token = "grpc-bootstrap".to_owned();
    let issuer = format!("http://{addr}");
    let oauth = OAuthState::new(OAuthConfig {
        issuer: issuer.clone(),
        client_id: "hivemind-cli".into(),
        client_secret: "grpc-client-secret".into(),
        signing_key: "grpc-signing-key".into(),
        resource: format!("{issuer}/mcp"),
        access_token_ttl_secs: DEFAULT_ACCESS_TOKEN_TTL_SECS,
        refresh_token_ttl_secs: DEFAULT_REFRESH_TOKEN_TTL_SECS,
    });

    let serve = tokio::spawn(grpc::serve(GrpcArgs {
        root: root.path().to_path_buf(),
        listen: addr,
        token: token.clone(),
        oauth,
        limiter: Arc::new(RateLimiter::new(RateLimitConfig {
            token_per_minute: 60,
            register_per_minute: 10,
            rpc_per_minute: 2,
        })),
        no_fsync: true,
    }));

    for _ in 0..50 {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let mut client = grpc::pb::hivemind_client::HivemindClient::connect(format!("http://{addr}"))
        .await
        .unwrap();

    let mut saw_ok = false;
    let mut saw_exhausted = false;
    for _ in 0..6 {
        let mut hello = Request::new(grpc::pb::HelloRequest {
            tenant: "acme".into(),
        });
        hello.metadata_mut().insert(
            "authorization",
            MetadataValue::try_from(format!("Bearer {token}")).unwrap(),
        );
        match client.hello(hello).await {
            Ok(_) => saw_ok = true,
            Err(status) => {
                if status.code() == tonic::Code::ResourceExhausted {
                    saw_exhausted = true;
                } else {
                    panic!("unexpected gRPC status: {status}");
                }
            }
        }
    }
    assert!(saw_ok, "expected some gRPC successes before limit");
    assert!(
        saw_exhausted,
        "expected ResourceExhausted after bursting gRPC RPC"
    );

    serve.abort();
}
