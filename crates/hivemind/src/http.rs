//! Shared-service HTTP API (M5): tenants, OAuth 2.1, MCP Streamable HTTP, backup.
//!
//! Endpoints:
//! - `/health` — open
//! - `/.well-known/oauth-*` and `/oauth/*` — OAuth 2.1 AS + protected resource
//! - `/mcp` — MCP Streamable HTTP for `--default-tenant`
//! - `/v1/tenants…` — JSON API (bearer: OAuth access token or bootstrap token)

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context as _, Result, anyhow};
use axum::Router;
use axum::extract::{Form, Path as AxumPath, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, middleware};
use hivemind_engine::{Durability, Options, Store};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::{StreamableHttpServerConfig, StreamableHttpService};
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::info;

use crate::mcp::HiveMcp;
use crate::oauth::{self, OAuthConfig, OAuthState, TokenForm};
use crate::service::Service;
use crate::sync;
use crate::tenant::{self, ensure_tenant_dir, list_tenants};

#[derive(Clone, Debug)]
pub struct ServeArgs {
    pub root: PathBuf,
    pub listen: SocketAddr,
    /// Bootstrap / admin bearer (also used as OAuth client secret + signing key).
    pub token: String,
    pub no_fsync: bool,
    /// Tenant backing `/mcp`.
    pub default_tenant: String,
    /// Public base URL for OAuth metadata (defaults to `http://{listen}`).
    pub public_base: Option<String>,
    pub oauth_client_id: String,
    /// Optional gRPC listen address (spawned alongside HTTP).
    pub grpc_listen: Option<SocketAddr>,
}

struct AppState {
    root: PathBuf,
    token: String,
    durability: Durability,
    oauth: OAuthState,
    stores: Mutex<HashMap<String, Arc<Service>>>,
}

pub async fn serve(args: ServeArgs) -> Result<()> {
    if args.token.trim().is_empty() {
        return Err(anyhow!("HIVEMIND_TOKEN / --token must be non-empty"));
    }
    std::fs::create_dir_all(args.root.join("tenants"))?;

    let issuer = args
        .public_base
        .clone()
        .unwrap_or_else(|| format!("http://{}", args.listen));
    let issuer = issuer.trim_end_matches('/').to_owned();
    let oauth = OAuthState::new(OAuthConfig {
        issuer: issuer.clone(),
        client_id: args.oauth_client_id.clone(),
        client_secret: args.token.clone(),
        signing_key: args.token.clone(),
        resource: format!("{issuer}/mcp"),
    });

    let state = Arc::new(AppState {
        root: args.root.clone(),
        token: args.token.clone(),
        durability: if args.no_fsync {
            Durability::NoFsync
        } else {
            Durability::Fsync
        },
        oauth: oauth.clone(),
        stores: Mutex::new(HashMap::new()),
    });

    // Ensure default tenant store exists before mounting MCP.
    let default_service = state
        .service_for(&args.default_tenant)
        .map_err(|e| anyhow!(e.message))?;
    let mcp = StreamableHttpService::new(
        {
            let default_service = default_service.clone();
            move || Ok(HiveMcp::from_service(default_service.clone()))
        },
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default()
            .with_allowed_hosts([
                "localhost".to_owned(),
                "127.0.0.1".to_owned(),
                "::1".to_owned(),
                args.listen.ip().to_string(),
                format!("{}:{}", args.listen.ip(), args.listen.port()),
                format!("localhost:{}", args.listen.port()),
                format!("127.0.0.1:{}", args.listen.port()),
            ]),
    );

    let protected = Router::new()
        .route("/v1/tenants", get(tenants_list))
        .route("/v1/tenants/{tenant}/hello", get(tenant_hello))
        .route("/v1/tenants/{tenant}/rpc", post(tenant_rpc))
        .route("/v1/tenants/{tenant}/changes", get(tenant_changes))
        .route("/v1/tenants/{tenant}/backup", post(tenant_backup))
        .nest_service("/mcp", mcp)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            require_bearer,
        ));

    let app = Router::new()
        .route("/health", get(health))
        .route(
            "/.well-known/oauth-authorization-server",
            get(oauth_as_metadata),
        )
        .route(
            "/.well-known/oauth-protected-resource",
            get(oauth_pr_metadata),
        )
        .route(
            "/.well-known/oauth-protected-resource/mcp",
            get(oauth_pr_metadata),
        )
        .route("/oauth/authorize", get(oauth_authorize))
        .route("/oauth/token", post(oauth_token))
        .merge(protected)
        .with_state(state.clone());

    if let Some(grpc_listen) = args.grpc_listen {
        let grpc_args = crate::grpc::GrpcArgs {
            root: args.root.clone(),
            listen: grpc_listen,
            token: args.token.clone(),
            oauth: oauth.clone(),
            no_fsync: args.no_fsync,
        };
        tokio::spawn(async move {
            if let Err(e) = crate::grpc::serve(grpc_args).await {
                tracing::error!(error = %e, "gRPC server exited");
            }
        });
        info!(%grpc_listen, "gRPC enabled");
    }

    let listener = tokio::net::TcpListener::bind(args.listen)
        .await
        .with_context(|| format!("bind {}", args.listen))?;
    info!(
        %args.listen,
        root = %args.root.display(),
        tenant = %args.default_tenant,
        issuer = %issuer,
        "hivemind HTTP service listening"
    );
    axum::serve(listener, app).await?;
    Ok(())
}

async fn require_bearer(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    request: axum::extract::Request,
    next: middleware::Next,
) -> Response {
    let header = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    match sync::bearer_token(header) {
        Ok(token) => match state.oauth.validate_bearer(token, &state.token) {
            Ok(()) => next.run(request).await,
            Err(e) => unauthorized(&state, e.to_string()),
        },
        Err(e) => unauthorized(&state, e.to_string()),
    }
}

fn unauthorized(state: &AppState, message: String) -> Response {
    let resource_meta = format!(
        "{}/.well-known/oauth-protected-resource",
        state.oauth.config.issuer
    );
    let www = format!(
        "Bearer realm=\"hivemind\", resource_metadata=\"{resource_meta}\""
    );
    (
        StatusCode::UNAUTHORIZED,
        [(axum::http::header::WWW_AUTHENTICATE, www)],
        Json(json!({"error": message})),
    )
        .into_response()
}

async fn health() -> Json<Value> {
    Json(json!({ "ok": true, "service": "hivemind" }))
}

async fn oauth_as_metadata(State(state): State<Arc<AppState>>) -> Json<Value> {
    Json(state.oauth.authorization_server_metadata())
}

async fn oauth_pr_metadata(State(state): State<Arc<AppState>>) -> Json<Value> {
    Json(state.oauth.protected_resource_metadata())
}

#[derive(Debug, Deserialize)]
struct AuthorizeQuery {
    response_type: Option<String>,
    client_id: Option<String>,
    redirect_uri: Option<String>,
    code_challenge: Option<String>,
    code_challenge_method: Option<String>,
    state: Option<String>,
    resource: Option<String>,
}

async fn oauth_authorize(
    State(state): State<Arc<AppState>>,
    Query(q): Query<AuthorizeQuery>,
) -> Response {
    if q.response_type.as_deref() != Some("code") {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"unsupported_response_type"})),
        )
            .into_response();
    }
    let client_id = q.client_id.unwrap_or_default();
    let redirect_uri = q.redirect_uri.unwrap_or_default();
    let challenge = q.code_challenge.unwrap_or_default();
    let method = q
        .code_challenge_method
        .unwrap_or_else(|| "S256".to_owned());
    if redirect_uri.is_empty() {
        return Html(
            "<!doctype html><title>hivemind oauth</title><p>redirect_uri required</p>",
        )
        .into_response();
    }
    match state.oauth.mint_authorize_code(
        &client_id,
        &redirect_uri,
        &challenge,
        &method,
        q.resource.as_deref(),
    ) {
        Ok(code) => {
            let mut url = format!("{redirect_uri}?code={code}");
            if let Some(s) = q.state {
                url.push_str("&state=");
                url.push_str(&urlencoding_minimal(&s));
            }
            Redirect::temporary(&url).into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"invalid_request","error_description": e.to_string()})),
        )
            .into_response(),
    }
}

async fn oauth_token(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Form(mut form): Form<TokenForm>,
) -> Result<Json<Value>, ApiError> {
    let basic = oauth::parse_basic_auth(
        headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok()),
    );
    oauth::merge_basic_into_form(&mut form, basic);
    let value = state.oauth.exchange_token(&form).map_err(ApiError::from)?;
    Ok(Json(value))
}

fn urlencoding_minimal(s: &str) -> String {
    // Enough for opaque state values in tests / local clients.
    s.replace('%', "%25")
        .replace('&', "%26")
        .replace('?', "%3F")
        .replace(' ', "%20")
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
