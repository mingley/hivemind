//! Shared-service HTTP API (M5): tenant partitions, bearer auth, change feed.
//!
//! This is the laptop/service JSON surface. MCP Streamable HTTP mounts beside
//! it when enabled. OAuth 2.1 can replace the shared bearer token later; the
//! wire shape stays `Authorization: Bearer …`.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context as _, Result, anyhow};
use axum::Router;
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, middleware};
use hivemind_engine::{Durability, Options, Store};
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::info;

use crate::service::Service;
use crate::sync;
use crate::tenant::{self, ensure_tenant_dir, list_tenants};

#[derive(Clone, Debug)]
pub struct ServeArgs {
    pub root: PathBuf,
    pub listen: SocketAddr,
    /// Shared bearer token (OAuth access tokens plug in the same header).
    pub token: String,
    pub no_fsync: bool,
}

struct AppState {
    root: PathBuf,
    token: String,
    durability: Durability,
    stores: Mutex<HashMap<String, Arc<Service>>>,
}

pub async fn serve(args: ServeArgs) -> Result<()> {
    if args.token.trim().is_empty() {
        return Err(anyhow!("HIVEMIND_TOKEN / --token must be non-empty"));
    }
    std::fs::create_dir_all(args.root.join("tenants"))?;
    let state = Arc::new(AppState {
        root: args.root.clone(),
        token: args.token,
        durability: if args.no_fsync {
            Durability::NoFsync
        } else {
            Durability::Fsync
        },
        stores: Mutex::new(HashMap::new()),
    });

    let app = Router::new()
        .route("/health", get(health))
        .route("/v1/tenants", get(tenants_list))
        .route("/v1/tenants/{tenant}/hello", get(tenant_hello))
        .route("/v1/tenants/{tenant}/rpc", post(tenant_rpc))
        .route("/v1/tenants/{tenant}/changes", get(tenant_changes))
        .route("/v1/tenants/{tenant}/backup", post(tenant_backup))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            require_bearer,
        ))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(args.listen)
        .await
        .with_context(|| format!("bind {}", args.listen))?;
    info!(%args.listen, root = %args.root.display(), "hivemind HTTP service listening");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn require_bearer(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    request: axum::extract::Request,
    next: middleware::Next,
) -> Response {
    let path = request.uri().path();
    if path == "/health" {
        return next.run(request).await;
    }
    let header = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    match sync::bearer_token(header) {
        Ok(token) if token == state.token => next.run(request).await,
        Ok(_) => (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"invalid token"})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn health() -> Json<Value> {
    Json(json!({ "ok": true, "service": "hivemind" }))
}

async fn tenants_list(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let tenants = list_tenants(&state.root).map_err(ApiError::from)?;
    Ok(Json(json!({ "tenants": tenants })))
}

async fn tenant_hello(
    State(state): State<Arc<AppState>>,
    AxumPath(tenant): AxumPath<String>,
) -> Result<Json<Value>, ApiError> {
    let service = state.service_for(&tenant)?;
    Ok(Json(
        serde_json::to_value(service.hello()).map_err(|e| ApiError::from(anyhow!(e)))?,
    ))
}

#[derive(Debug, Deserialize)]
struct RpcBody {
    method: String,
    #[serde(default)]
    params: Value,
}

async fn tenant_rpc(
    State(state): State<Arc<AppState>>,
    AxumPath(tenant): AxumPath<String>,
    Json(body): Json<RpcBody>,
) -> Result<Json<Value>, ApiError> {
    let service = state.service_for(&tenant)?;
    let value = service
        .dispatch(&body.method, body.params)
        .map_err(ApiError::from)?;
    Ok(Json(value))
}

#[derive(Debug, Deserialize)]
struct ChangesQuery {
    #[serde(default)]
    from: u64,
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_limit() -> usize {
    100
}

async fn tenant_changes(
    State(state): State<Arc<AppState>>,
    AxumPath(tenant): AxumPath<String>,
    Query(q): Query<ChangesQuery>,
) -> Result<Json<Value>, ApiError> {
    let service = state.service_for(&tenant)?;
    let value = service
        .dispatch("changes_since", json!({ "from": q.from, "limit": q.limit }))
        .map_err(ApiError::from)?;
    Ok(Json(value))
}

#[derive(Debug, Deserialize)]
struct BackupBody {
    dest: PathBuf,
}

async fn tenant_backup(
    State(state): State<Arc<AppState>>,
    AxumPath(tenant): AxumPath<String>,
    Json(body): Json<BackupBody>,
) -> Result<Json<Value>, ApiError> {
    let service = state.service_for(&tenant)?;
    let report = service
        .store()
        .backup_segments(&body.dest)
        .map_err(|e| ApiError::from(anyhow!(e)))?;
    Ok(Json(
        serde_json::to_value(report).map_err(|e| ApiError::from(anyhow!(e)))?,
    ))
}

impl AppState {
    fn service_for(&self, tenant: &str) -> Result<Arc<Service>, ApiError> {
        tenant::validate_tenant_id(tenant).map_err(ApiError::from)?;
        {
            let guard = self
                .stores
                .lock()
                .map_err(|_| ApiError::from(anyhow!("lock poisoned")))?;
            if let Some(svc) = guard.get(tenant) {
                return Ok(svc.clone());
            }
        }
        let dir = ensure_tenant_dir(&self.root, tenant).map_err(ApiError::from)?;
        let store = Store::open(
            &dir,
            Options {
                durability: self.durability,
                ..Options::default()
            },
        )
        .map_err(|e| ApiError::from(anyhow!(e)))?;
        let service = Arc::new(Service::new(store));
        let mut guard = self
            .stores
            .lock()
            .map_err(|_| ApiError::from(anyhow!("lock poisoned")))?;
        Ok(guard.entry(tenant.to_owned()).or_insert(service).clone())
    }
}

struct ApiError {
    status: StatusCode,
    message: String,
}

impl From<anyhow::Error> for ApiError {
    fn from(value: anyhow::Error) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: value.to_string(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.message }))).into_response()
    }
}

/// Open (or create) the store for a tenant under `root` — used by CLI helpers.
pub fn open_tenant_store(root: &Path, tenant: &str, durability: Durability) -> Result<Store> {
    let dir = ensure_tenant_dir(root, tenant)?;
    Store::open(
        &dir,
        Options {
            durability,
            ..Options::default()
        },
    )
    .map_err(Into::into)
}
