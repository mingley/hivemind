//! Shared-service HTTP API (M5+): tenants, OAuth 2.1, MCP Streamable HTTP, backup.
//!
//! Endpoints:
//! - `/health` — liveness (process up; existing probe, kept stable)
//! - `/readyz` — readiness (default tenant store open; additive)
//! - `/metrics` — Prometheus text exposition (additive)
//! - `/healthz` — optional alias of `/health` (not a required rename)
//! - `/.well-known/oauth-*` and `/oauth/*` — OAuth 2.1 AS + protected resource
//! - `/mcp` — MCP Streamable HTTP for `--default-tenant`
//! - `/v1/tenants…` — JSON API (bearer: OAuth access token or bootstrap token)
//!
//! Rate limits (in-process token buckets): `/oauth/token` and authenticated
//! RPC/MCP paths. Ops probes above are exempt.
//!
//! Transport: this process speaks plaintext HTTP. Production deployments MUST
//! terminate TLS at a reverse proxy (or equivalent) and set
//! `HIVEMIND_REQUIRE_TLS=1` so a non-`https://` public base is refused. See
//! repository root `SECURITY.md`.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::{Context as _, Result, anyhow};
use axum::Router;
use axum::extract::{ConnectInfo, Extension, Form, Path as AxumPath, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::middleware::Next;
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
use crate::oauth::{
    self, AuthIdentity, DEFAULT_ACCESS_TOKEN_TTL_SECS, OAuthConfig, OAuthState, TokenForm,
    validate_transport,
};
use crate::obs;
use crate::ratelimit::{self, RateClass, RateLimitConfig, RateLimited, RateLimiter};
use crate::service::Service;
use crate::sync;
use crate::tenant::{self, ensure_tenant_dir, list_tenants};

#[derive(Clone, Debug)]
pub struct ServeArgs {
    pub root: PathBuf,
    pub listen: SocketAddr,
    /// Bootstrap / admin bearer (not the OAuth client secret or signing key).
    pub token: String,
    /// Pre-registered confidential client secret.
    pub oauth_client_secret: String,
    /// HMAC key for sealing access tokens.
    pub signing_key: String,
    pub no_fsync: bool,
    /// Tenant backing `/mcp`.
    pub default_tenant: String,
    /// Public base URL for OAuth metadata (defaults to `http://{listen}`).
    pub public_base: Option<String>,
    pub oauth_client_id: String,
    /// Access-token TTL seconds (short by default).
    pub access_token_ttl_secs: u64,
    /// Refresh-token TTL seconds (authorization_code / refresh_token grants).
    pub refresh_token_ttl_secs: u64,
    /// When true, refuse to start unless `public_base` is `https://`.
    pub require_tls: bool,
    /// Optional gRPC listen address (spawned alongside HTTP).
    pub grpc_listen: Option<SocketAddr>,
    /// Sustained `/oauth/token` requests per minute per IP+client_id (`0` = off).
    pub token_rate_per_minute: u64,
    /// Sustained authenticated RPC requests per minute per IP+principal (`0` = off).
    pub rpc_rate_per_minute: u64,
}

struct AppState {
    root: PathBuf,
    token: String,
    durability: Durability,
    oauth: OAuthState,
    default_tenant: String,
    stores: Mutex<HashMap<String, Arc<Service>>>,
    limiter: Arc<RateLimiter>,
}

pub async fn serve(args: ServeArgs) -> Result<()> {
    obs::init_tracing();
    obs::ensure_registered();
    validate_serve_secrets(&args)?;
    std::fs::create_dir_all(args.root.join("tenants"))?;

    let issuer = args
        .public_base
        .clone()
        .unwrap_or_else(|| format!("http://{}", args.listen));
    let issuer = issuer.trim_end_matches('/').to_owned();
    validate_transport(args.require_tls, &issuer)?;

    let oauth = OAuthState::new(OAuthConfig {
        issuer: issuer.clone(),
        client_id: args.oauth_client_id.clone(),
        client_secret: args.oauth_client_secret.clone(),
        signing_key: args.signing_key.clone(),
        resource: format!("{issuer}/mcp"),
        access_token_ttl_secs: args.access_token_ttl_secs,
        refresh_token_ttl_secs: args.refresh_token_ttl_secs,
    });

    let limiter = Arc::new(RateLimiter::new(RateLimitConfig {
        token_per_minute: args.token_rate_per_minute,
        rpc_per_minute: args.rpc_rate_per_minute,
    }));

    let state = Arc::new(AppState {
        root: args.root.clone(),
        token: args.token.clone(),
        durability: if args.no_fsync {
            Durability::NoFsync
        } else {
            Durability::Fsync
        },
        oauth: oauth.clone(),
        default_tenant: args.default_tenant.clone(),
        stores: Mutex::new(HashMap::new()),
        limiter: limiter.clone(),
    });

    // Ensure default tenant store exists before mounting MCP.
    let default_service = state
        .service_for(&args.default_tenant)
        .map_err(|e| anyhow!(e.message))?;
    // Streamable MCP cannot see per-request auth extensions; attribute writes
    // to the pre-registered OAuth client_id (not env TOFU).
    let mcp_principal = args.oauth_client_id.clone();
    let mcp = StreamableHttpService::new(
        {
            let default_service = default_service.clone();
            let mcp_principal = mcp_principal.clone();
            move || {
                Ok(HiveMcp::from_service_with_principal(
                    default_service.clone(),
                    mcp_principal.clone(),
                ))
            }
        },
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default().with_allowed_hosts([
            "localhost".to_owned(),
            "127.0.0.1".to_owned(),
            "::1".to_owned(),
            args.listen.ip().to_string(),
            format!("{}:{}", args.listen.ip(), args.listen.port()),
            format!("localhost:{}", args.listen.port()),
            format!("127.0.0.1:{}", args.listen.port()),
        ]),
    );

    // Layers apply outermost-last on the request path: bearer runs before RPC limit.
    let protected = Router::new()
        .route("/v1/tenants", get(tenants_list))
        .route("/v1/tenants/{tenant}/hello", get(tenant_hello))
        .route("/v1/tenants/{tenant}/rpc", post(tenant_rpc))
        .route("/v1/tenants/{tenant}/changes", get(tenant_changes))
        .route("/v1/tenants/{tenant}/backup", post(tenant_backup))
        .nest_service("/mcp", mcp)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            rate_limit_rpc,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            require_bearer,
        ));

    // Ops probes stay open (no rate-limit layer). Token endpoint is limited in-handler.
    let app = Router::new()
        .route("/health", get(health))
        .route("/readyz", get(readyz))
        .route("/metrics", get(metrics))
        .route("/healthz", get(health))
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
        .layer(middleware::from_fn(http_metrics))
        .with_state(state.clone());

    if let Some(grpc_listen) = args.grpc_listen {
        let grpc_args = crate::grpc::GrpcArgs {
            root: args.root.clone(),
            listen: grpc_listen,
            token: args.token.clone(),
            oauth: oauth.clone(),
            limiter: limiter.clone(),
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
        require_tls = args.require_tls,
        token_rate_per_minute = args.token_rate_per_minute,
        rpc_rate_per_minute = args.rpc_rate_per_minute,
        "hivemind HTTP service listening"
    );
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

/// Reject empty or accidentally dual-used secrets before bind.
pub fn validate_serve_secrets(args: &ServeArgs) -> Result<()> {
    if args.token.trim().is_empty() {
        return Err(anyhow!("HIVEMIND_TOKEN / --token must be non-empty"));
    }
    if args.oauth_client_secret.trim().is_empty() {
        return Err(anyhow!(
            "HIVEMIND_OAUTH_CLIENT_SECRET / --oauth-client-secret must be non-empty"
        ));
    }
    if args.signing_key.trim().is_empty() {
        return Err(anyhow!(
            "HIVEMIND_SIGNING_KEY / --signing-key must be non-empty"
        ));
    }
    if args.token == args.oauth_client_secret {
        return Err(anyhow!(
            "bootstrap token and OAuth client secret must be distinct"
        ));
    }
    if args.token == args.signing_key {
        return Err(anyhow!("bootstrap token and signing key must be distinct"));
    }
    if args.oauth_client_secret == args.signing_key {
        return Err(anyhow!(
            "OAuth client secret and signing key must be distinct"
        ));
    }
    if args.access_token_ttl_secs == 0 {
        return Err(anyhow!("access token TTL must be > 0"));
    }
    if args.refresh_token_ttl_secs == 0 {
        return Err(anyhow!("refresh token TTL must be > 0"));
    }
    Ok(())
}

async fn http_metrics(request: axum::extract::Request, next: Next) -> Response {
    let method = request.method().to_string();
    let path = obs::normalize_path(request.uri().path());
    let started = Instant::now();
    let response = next.run(request).await;
    obs::record_http(&method, &path, response.status().as_u16(), started);
    response
}

async fn require_bearer(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    mut request: axum::extract::Request,
    next: middleware::Next,
) -> Response {
    let header = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    match sync::bearer_token(header) {
        Ok(token) => match state.oauth.authenticate(token, &state.token) {
            Ok(identity) => {
                request.extensions_mut().insert(identity);
                next.run(request).await
            }
            Err(e) => unauthorized(&state, e.to_string()),
        },
        Err(e) => unauthorized(&state, e.to_string()),
    }
}

async fn rate_limit_rpc(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    request: axum::extract::Request,
    next: middleware::Next,
) -> Response {
    let principal = request
        .extensions()
        .get::<AuthIdentity>()
        .map(|id| id.principal.as_str())
        .unwrap_or("anonymous");
    let key = ratelimit::peer_key(&addr.ip().to_string(), principal);
    match state.limiter.check(RateClass::Rpc, &key) {
        Ok(()) => next.run(request).await,
        Err(limited) => too_many_requests(limited),
    }
}

fn unauthorized(state: &AppState, message: String) -> Response {
    let resource_meta = format!(
        "{}/.well-known/oauth-protected-resource",
        state.oauth.config.issuer
    );
    let www = format!("Bearer realm=\"hivemind\", resource_metadata=\"{resource_meta}\"");
    (
        StatusCode::UNAUTHORIZED,
        [(axum::http::header::WWW_AUTHENTICATE, www)],
        Json(json!({"error": message})),
    )
        .into_response()
}

fn too_many_requests(limited: RateLimited) -> Response {
    (
        StatusCode::TOO_MANY_REQUESTS,
        [(
            axum::http::header::RETRY_AFTER,
            HeaderValue::from_str(&limited.retry_after_secs.to_string())
                .unwrap_or_else(|_| HeaderValue::from_static("1")),
        )],
        Json(json!({
            "error": "rate_limited",
            "error_description": "too many requests; slow down",
            "retry_after_secs": limited.retry_after_secs,
        })),
    )
        .into_response()
}

async fn health() -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(json!({ "ok": true, "service": "hivemind", "probe": "liveness" })),
    )
}

async fn readyz(State(state): State<Arc<AppState>>) -> Response {
    match state.service_for(&state.default_tenant) {
        Ok(service) => {
            let offset = service.store().next_offset();
            obs::set_log_offset(&state.default_tenant, offset);
            (
                StatusCode::OK,
                Json(json!({
                    "ok": true,
                    "service": "hivemind",
                    "probe": "readiness",
                    "tenant": state.default_tenant,
                    "next_offset": offset,
                })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "ok": false,
                "service": "hivemind",
                "probe": "readiness",
                "error": e.message,
            })),
        )
            .into_response(),
    }
}

async fn metrics(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    state.refresh_log_offsets();
    let body = obs::render_prometheus();
    (
        StatusCode::OK,
        [(
            axum::http::header::CONTENT_TYPE,
            HeaderValue::from_static("text/plain; version=0.0.4; charset=utf-8"),
        )],
        body,
    )
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
    let method = q.code_challenge_method.unwrap_or_else(|| "S256".to_owned());
    if redirect_uri.is_empty() {
        return Html("<!doctype html><title>hivemind oauth</title><p>redirect_uri required</p>")
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
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Form(mut form): Form<TokenForm>,
) -> Result<Json<Value>, ApiError> {
    let basic = oauth::parse_basic_auth(
        headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok()),
    );
    oauth::merge_basic_into_form(&mut form, basic);
    let client_id = form.client_id.as_deref().unwrap_or("unknown");
    let key = ratelimit::peer_key(&addr.ip().to_string(), client_id);
    if let Err(limited) = state.limiter.check(RateClass::Token, &key) {
        return Err(ApiError::rate_limited(limited));
    }
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
    Extension(identity): Extension<AuthIdentity>,
    AxumPath(tenant): AxumPath<String>,
    Json(body): Json<RpcBody>,
) -> Result<Json<Value>, ApiError> {
    let service = state.service_for(&tenant)?;
    let value = service
        .dispatch_with_principal(&body.method, body.params, Some(&identity.principal))
        .map_err(ApiError::from)?;
    obs::set_log_offset(&tenant, service.store().next_offset());
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
                obs::set_log_offset(tenant, svc.store().next_offset());
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
        let service = guard.entry(tenant.to_owned()).or_insert(service).clone();
        obs::set_log_offset(tenant, service.store().next_offset());
        Ok(service)
    }

    fn refresh_log_offsets(&self) {
        let Ok(guard) = self.stores.lock() else {
            return;
        };
        for (tenant, service) in guard.iter() {
            obs::set_log_offset(tenant, service.store().next_offset());
        }
    }
}

struct ApiError {
    status: StatusCode,
    message: String,
    retry_after_secs: Option<u64>,
}

impl ApiError {
    fn rate_limited(limited: RateLimited) -> Self {
        Self {
            status: StatusCode::TOO_MANY_REQUESTS,
            message: "rate_limited".into(),
            retry_after_secs: Some(limited.retry_after_secs),
        }
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(value: anyhow::Error) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: value.to_string(),
            retry_after_secs: None,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = if self.status == StatusCode::TOO_MANY_REQUESTS {
            json!({
                "error": "rate_limited",
                "error_description": "too many requests; slow down",
                "retry_after_secs": self.retry_after_secs.unwrap_or(1),
            })
        } else {
            json!({ "error": self.message })
        };
        let mut response = (self.status, Json(body)).into_response();
        if let Some(secs) = self.retry_after_secs
            && let Ok(value) = HeaderValue::from_str(&secs.to_string())
        {
            response
                .headers_mut()
                .insert(axum::http::header::RETRY_AFTER, value);
        }
        response
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

/// Helper for tests / docs: default short TTL.
pub fn default_access_token_ttl_secs() -> u64 {
    DEFAULT_ACCESS_TOKEN_TTL_SECS
}
