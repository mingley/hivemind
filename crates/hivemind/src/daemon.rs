//! `hivemindd`: owns a store and serves the Unix-socket protocol.

use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write as _};
use std::net::SocketAddr;
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, anyhow, bail};
use axum::extract::State as AxumState;
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response as HttpResponse};
use axum::routing::{get, post};
use axum::{Json, Router};
use clap::Parser;
use hivemind_engine::{Durability, Options, Store};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::{StreamableHttpServerConfig, StreamableHttpService};
use sha2::{Digest, Sha256};
use tracing::{info, warn};

use crate::embedder::EmbedderArgs;
use crate::mcp::HiveMcp;
use crate::oauth::AuthIdentity;
use crate::protocol::{self, RUN_DIR, Request, Response};
use crate::service::Service;
use crate::sync;

const CLIENT_POLL_READ_TIMEOUT: Duration = Duration::from_secs(1);
const PARTIAL_FRAME_TIMEOUT: Duration = Duration::from_secs(30);
pub const DEFAULT_MCP_HTTP_TOKEN_FILE: &str = "mcp-http.token";

#[derive(Clone, Parser, Debug)]
#[command(name = "hivemindd", version, about = "hivemind store daemon")]
pub struct DaemonArgs {
    /// Store directory [default: ~/.hivemind/store].
    #[arg(long, env = "HIVEMIND_DIR")]
    pub dir: Option<PathBuf>,
    /// Seconds with no clients before exit. 0 disables idle exit.
    #[arg(long, env = "HIVEMIND_IDLE_TIMEOUT", default_value_t = 600)]
    pub idle_timeout: u64,
    /// Optional loopback-only Streamable HTTP `/mcp` + `/rpc` listener (e.g. 127.0.0.1:8787)
    /// for SSH reverse-tunneled dev environments sharing this store.
    #[arg(long, env = "HIVEMIND_MCP_HTTP")]
    pub mcp_http: Option<SocketAddr>,
    /// Bearer token file (`0600`) for `--mcp-http` [default: <dir>/mcp-http.token].
    #[arg(long, env = "HIVEMIND_MCP_HTTP_TOKEN_FILE")]
    pub mcp_http_token_file: Option<PathBuf>,
    /// Skip fsync (tests/benchmarks only).
    #[arg(long, hide = true)]
    pub no_fsync: bool,
    /// Stay in the foreground (default). Background spawners redirect stdio.
    #[arg(long, default_value_t = true)]
    pub foreground: bool,
    /// Spawned in a detached process group; ignore SIGHUP so terminal close
    /// does not tear down the shared daemon for other sessions.
    #[arg(long, hide = true)]
    pub detached: bool,
    #[command(flatten)]
    pub embedder: EmbedderArgs,
}

pub fn main_from_env() -> Result<()> {
    let args = DaemonArgs::parse();
    run(args)
}

pub fn run(args: DaemonArgs) -> Result<()> {
    crate::obs::init_tracing();
    let dir = match args.dir {
        Some(d) => d,
        None => default_dir()?,
    };
    if let Some(addr) = args.mcp_http {
        validate_loopback_mcp_addr(addr)?;
    }
    let durability = if args.no_fsync {
        Durability::NoFsync
    } else {
        Durability::Fsync
    };
    let options = Options {
        durability,
        ..Options::default()
    }
    .with_env_overrides()
    .context("HIVEMIND_EVENT_CODEC")?;
    let store =
        Store::open(&dir, options).with_context(|| format!("open store {}", dir.display()))?;
    args.embedder
        .apply(&store)
        .context("attach optional embedder")?;
    let service = Arc::new(Service::new(store));
    let run_dir = dir.join(RUN_DIR);
    fs::create_dir_all(&run_dir)?;
    let mut perms = fs::metadata(&run_dir)?.permissions();
    perms.set_mode(0o700);
    fs::set_permissions(&run_dir, perms)?;

    let sock_path = protocol::socket_path(&dir);
    // Stale socket from a previous crash: safe to remove now that we hold LOCK.
    match fs::remove_file(&sock_path) {
        Ok(()) => {}
        Err(err) if err.kind() == ErrorKind::NotFound => {}
        Err(err) => return Err(err).context("remove stale socket"),
    }
    let listener =
        UnixListener::bind(&sock_path).with_context(|| format!("bind {}", sock_path.display()))?;
    fs::set_permissions(&sock_path, fs::Permissions::from_mode(0o600))?;
    // Ensure the socket file is removed when we exit.
    let _guard = SocketGuard(sock_path.clone());

    let shutdown = Arc::new(AtomicBool::new(false));
    #[cfg(unix)]
    spawn_signal_watcher(args.detached, Arc::clone(&shutdown))?;

    let last_activity = Arc::new(AtomicU64::new(now_secs()));
    let clients = Arc::new(AtomicU64::new(0));

    let mcp_http_server = if let Some(addr) = args.mcp_http {
        let token_path = args
            .mcp_http_token_file
            .clone()
            .unwrap_or_else(|| dir.join(DEFAULT_MCP_HTTP_TOKEN_FILE));
        let token = load_or_create_mcp_token(&token_path)
            .with_context(|| format!("prepare mcp-http token {}", token_path.display()))?;
        let principal = std::env::var("HIVEMIND_PRINCIPAL")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| std::env::var("USER").ok().filter(|s| !s.trim().is_empty()))
            .unwrap_or_else(|| "local".into());
        let (bound_addr, handle) = spawn_loopback_mcp_http(
            addr,
            token,
            principal,
            Arc::clone(&service),
            Arc::clone(&last_activity),
            Arc::clone(&clients),
            Arc::clone(&shutdown),
        )?;
        info!(
            mcp_http = %bound_addr,
            token_file = %token_path.display(),
            "hivemindd loopback MCP HTTP ready"
        );
        Some(handle)
    } else {
        None
    };

    info!(
        store = %dir.display(),
        socket = %sock_path.display(),
        idle_timeout = args.idle_timeout,
        detached = args.detached,
        "hivemindd ready"
    );

    listener.set_nonblocking(true)?;

    loop {
        if shutdown.load(Ordering::Relaxed) {
            info!("shutdown requested; stopping accept loop");
            break;
        }
        match listener.accept() {
            Ok((stream, _)) => {
                last_activity.store(now_secs(), Ordering::Relaxed);
                clients.fetch_add(1, Ordering::Relaxed);
                let service = Arc::clone(&service);
                let last_activity = Arc::clone(&last_activity);
                let clients = Arc::clone(&clients);
                let shutdown = Arc::clone(&shutdown);
                thread::spawn(move || {
                    let _client_guard = ClientGuard {
                        clients: Arc::clone(&clients),
                        last_activity: Arc::clone(&last_activity),
                    };
                    if let Err(err) = handle_client(stream, &service, &shutdown) {
                        warn!("client error: {err:#}");
                    }
                });
            }
            Err(err) if err.kind() == ErrorKind::WouldBlock => {
                if args.idle_timeout > 0
                    && clients.load(Ordering::Relaxed) == 0
                    && now_secs().saturating_sub(last_activity.load(Ordering::Relaxed))
                        >= args.idle_timeout
                {
                    info!("idle timeout; exiting");
                    break;
                }
                thread::sleep(Duration::from_millis(50));
            }
            Err(err) => return Err(err).context("accept"),
        }
    }

    // Signal auxiliary listeners (such as --mcp-http) to stop on exit.
    shutdown.store(true, Ordering::Relaxed);

    // Give active handlers a brief window to finish in-flight responses cleanly.
    let drain_deadline = Instant::now() + Duration::from_secs(2);
    while clients.load(Ordering::Relaxed) > 0 && Instant::now() < drain_deadline {
        thread::sleep(Duration::from_millis(20));
    }
    if let Some(handle) = mcp_http_server {
        let _ = handle.join();
    }
    Ok(())
}

#[cfg(unix)]
fn spawn_signal_watcher(detached: bool, shutdown: Arc<AtomicBool>) -> Result<()> {
    use tokio::signal::unix::{SignalKind, signal};

    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel::<Result<()>>(1);
    thread::Builder::new()
        .name("hivemindd-signals".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(err) => {
                    let _ = ready_tx.send(Err(err.into()));
                    return;
                }
            };
            rt.block_on(async move {
                let mut sigterm = match signal(SignalKind::terminate()) {
                    Ok(s) => s,
                    Err(err) => {
                        let _ = ready_tx.send(Err(err.into()));
                        return;
                    }
                };
                let mut sigint = match signal(SignalKind::interrupt()) {
                    Ok(s) => s,
                    Err(err) => {
                        let _ = ready_tx.send(Err(err.into()));
                        return;
                    }
                };
                let mut sighup = if detached {
                    match signal(SignalKind::hangup()) {
                        Ok(s) => Some(s),
                        Err(err) => {
                            let _ = ready_tx.send(Err(err.into()));
                            return;
                        }
                    }
                } else {
                    None
                };
                let _ = ready_tx.send(Ok(()));
                loop {
                    tokio::select! {
                        _ = sigterm.recv() => {
                            shutdown.store(true, Ordering::Relaxed);
                            break;
                        }
                        _ = sigint.recv() => {
                            shutdown.store(true, Ordering::Relaxed);
                            break;
                        }
                        _ = async {
                            if let Some(hup) = sighup.as_mut() {
                                let _ = hup.recv().await;
                            } else {
                                std::future::pending::<()>().await;
                            }
                        } => {
                            // Detached daemon ignores SIGHUP so terminal close
                            // does not break active sessions.
                        }
                    }
                }
            });
        })
        .context("spawn signal watcher")?;
    ready_rx
        .recv()
        .map_err(|_| anyhow!("signal watcher exited before registering handlers"))??;
    Ok(())
}

struct ClientGuard {
    clients: Arc<AtomicU64>,
    last_activity: Arc<AtomicU64>,
}

impl Drop for ClientGuard {
    fn drop(&mut self) {
        self.clients.fetch_sub(1, Ordering::Relaxed);
        self.last_activity.store(now_secs(), Ordering::Relaxed);
    }
}

fn handle_client(stream: UnixStream, service: &Service, shutdown: &AtomicBool) -> Result<()> {
    use std::io::{BufRead, BufReader};
    // BSD/macOS may inherit O_NONBLOCK from the accepting listener. Each
    // connection has its own blocking worker; WouldBlock must not close it.
    stream.set_nonblocking(false)?;
    // Use a short read timeout so idle sessions stay connected indefinitely
    // while still noticing daemon shutdown within ~1s.
    stream.set_read_timeout(Some(CLIENT_POLL_READ_TIMEOUT))?;
    stream.set_write_timeout(Some(Duration::from_secs(30)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut writer = stream;
    let mut line = String::new();
    let mut partial_started: Option<Instant> = None;
    loop {
        line.clear();
        loop {
            if shutdown.load(Ordering::Relaxed) && line.is_empty() {
                return Ok(());
            }
            match reader.read_line(&mut line) {
                Ok(0) => return Ok(()),
                Ok(_) => {
                    partial_started = None;
                    break;
                }
                Err(err) if matches!(err.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                    if line.is_empty() {
                        continue;
                    }
                    let started = partial_started.get_or_insert_with(Instant::now);
                    if started.elapsed() >= PARTIAL_FRAME_TIMEOUT {
                        return Err(anyhow!("timed out reading partial request"));
                    }
                }
                Err(err) => return Err(err.into()),
            }
        }
        let req: Request = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(err) => {
                let resp = Response::err(0, "invalid", format!("bad request: {err}"));
                write_response(&mut writer, &resp)?;
                continue;
            }
        };
        if req.method == "shutdown" {
            shutdown.store(true, Ordering::Relaxed);
            let resp = Response::ok(req.id, serde_json::json!({"stopping": true}));
            write_response(&mut writer, &resp)?;
            break;
        }
        if req.method == "backup" {
            let resp = match req
                .params
                .get("dest")
                .and_then(|v| v.as_str())
                .map(PathBuf::from)
            {
                Some(dest) => match service.store().backup_segments(&dest) {
                    Ok(report) => Response::ok(req.id, report),
                    Err(err) => {
                        let err = anyhow::Error::from(err);
                        Response::err(req.id, classify(&err), format!("{err:#}"))
                    }
                },
                None => Response::err(req.id, "invalid", "backup requires `dest` path"),
            };
            write_response(&mut writer, &resp)?;
            continue;
        }
        let resp = match service.dispatch(&req.method, req.params) {
            Ok(result) => Response {
                id: req.id,
                ok: true,
                result: Some(result),
                error: None,
            },
            Err(err) => Response::err(req.id, classify(&err), format!("{err:#}")),
        };
        write_response(&mut writer, &resp)?;
    }
    Ok(())
}

fn write_response(writer: &mut UnixStream, resp: &Response) -> Result<()> {
    use std::io::Write;
    let mut line = serde_json::to_string(resp)?;
    line.push('\n');
    writer.write_all(line.as_bytes())?;
    writer.flush()?;
    Ok(())
}

fn classify(err: &anyhow::Error) -> &'static str {
    let msg = format!("{err:#}");
    if msg.contains("not found") {
        "not_found"
    } else if msg.contains("conflict") {
        "conflict"
    } else if msg.contains("timed out") {
        "timeout"
    } else if msg.contains("secret") {
        "secret"
    } else {
        "error"
    }
}

fn default_dir() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").ok_or_else(|| anyhow!("set HOME or pass --dir"))?;
    Ok(PathBuf::from(home).join(".hivemind").join("store"))
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

struct SocketGuard(PathBuf);

impl Drop for SocketGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

pub fn validate_loopback_mcp_addr(addr: SocketAddr) -> Result<()> {
    if !addr.ip().is_loopback() {
        bail!(
            "--mcp-http on hivemindd only accepts loopback addresses (127.0.0.1 / ::1), got {addr}; use `hivemind serve` for network listeners"
        );
    }
    Ok(())
}

pub fn load_or_create_mcp_token(path: &Path) -> Result<String> {
    if let Ok(meta) = fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() {
            bail!("refusing symlink mcp-http token file {}", path.display());
        }
        let existing = fs::read_to_string(path)?;
        let trimmed = existing.trim();
        if trimmed.len() >= 32 && !trimmed.chars().any(|c| c.is_whitespace() || c.is_control()) {
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
            return Ok(trimmed.to_owned());
        }
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut raw = [0u8; 32];
    getrandom::fill(&mut raw).map_err(|e| anyhow!("generate mcp-http token: {e}"))?;
    let mut token = String::with_capacity(64);
    for b in raw {
        use std::fmt::Write as _;
        let _ = write!(token, "{b:02x}");
    }
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    let write_res = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        file.write_all(token.as_bytes())?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        fs::rename(&tmp, path)?;
        Ok(())
    })();
    if write_res.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    write_res?;
    Ok(token)
}

struct LoopbackHttpState {
    token_sha256: [u8; 32],
    principal: String,
    service: Arc<Service>,
    last_activity: Arc<AtomicU64>,
    clients: Arc<AtomicU64>,
}

fn spawn_loopback_mcp_http(
    addr: SocketAddr,
    token: String,
    principal: String,
    service: Arc<Service>,
    last_activity: Arc<AtomicU64>,
    clients: Arc<AtomicU64>,
    shutdown: Arc<AtomicBool>,
) -> Result<(SocketAddr, thread::JoinHandle<()>)> {
    let token_sha256: [u8; 32] = Sha256::digest(token.as_bytes()).into();
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel::<Result<SocketAddr>>(1);

    let handle = thread::Builder::new()
        .name("hivemindd-mcp-http".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(err) => {
                    let _ = ready_tx.send(Err(err.into()));
                    return;
                }
            };
            rt.block_on(async move {
                let listener = match tokio::net::TcpListener::bind(addr).await {
                    Ok(l) => l,
                    Err(err) => {
                        let _ = ready_tx.send(Err(anyhow!("bind mcp-http {addr}: {err}")));
                        return;
                    }
                };
                let bound = match listener.local_addr() {
                    Ok(b) => b,
                    Err(err) => {
                        let _ = ready_tx.send(Err(err.into()));
                        return;
                    }
                };

                let state = Arc::new(LoopbackHttpState {
                    token_sha256,
                    principal: principal.clone(),
                    service: Arc::clone(&service),
                    last_activity,
                    clients,
                });

                let mcp = StreamableHttpService::new(
                    {
                        let service = Arc::clone(&service);
                        let principal = principal.clone();
                        move || {
                            Ok(HiveMcp::from_service_with_principal(
                                Arc::clone(&service),
                                principal.clone(),
                            ))
                        }
                    },
                    Arc::new(LocalSessionManager::default()),
                    StreamableHttpServerConfig::default().with_allowed_hosts([
                        "localhost".to_owned(),
                        "127.0.0.1".to_owned(),
                        "::1".to_owned(),
                        "[::1]".to_owned(),
                        bound.ip().to_string(),
                        format!("{}:{}", bound.ip(), bound.port()),
                        format!("localhost:{}", bound.port()),
                        format!("127.0.0.1:{}", bound.port()),
                        format!("[::1]:{}", bound.port()),
                    ]),
                );

                let protected = Router::new()
                    .route("/rpc", post(loopback_rpc))
                    .nest_service("/mcp", mcp)
                    .layer(middleware::from_fn_with_state(
                        Arc::clone(&state),
                        require_loopback_bearer,
                    ));

                let app = Router::new()
                    .route("/health", get(loopback_health))
                    .merge(protected)
                    .with_state(state);

                let _ = ready_tx.send(Ok(bound));

                let shutdown_signal = async move {
                    while !shutdown.load(Ordering::Relaxed) {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                };
                if let Err(err) = axum::serve(listener, app)
                    .with_graceful_shutdown(shutdown_signal)
                    .await
                {
                    warn!("mcp-http server error: {err:#}");
                }
            });
        })
        .context("spawn mcp-http thread")?;

    let bound = ready_rx
        .recv()
        .map_err(|_| anyhow!("mcp-http thread exited before bind"))??;
    Ok((bound, handle))
}

async fn loopback_health(AxumState(state): AxumState<Arc<LoopbackHttpState>>) -> impl IntoResponse {
    let hello = state.service.hello();
    Json(serde_json::json!({
        "ok": true,
        "protocol": "hivemind-loopback-mcp-v1",
        "version": hello.version,
        "store_id": hello.store_id,
        "next_offset": hello.next_offset,
    }))
}

async fn require_loopback_bearer(
    AxumState(state): AxumState<Arc<LoopbackHttpState>>,
    headers: HeaderMap,
    mut request: axum::extract::Request,
    next: Next,
) -> HttpResponse {
    let raw_auth = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    let Ok(provided) = sync::bearer_token(raw_auth) else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "missing or invalid bearer token"})),
        )
            .into_response();
    };
    let provided_sha: [u8; 32] = Sha256::digest(provided.as_bytes()).into();
    // Constant-time 32-byte digest comparison (no early exit).
    let mut diff = 0u8;
    for (a, b) in provided_sha.iter().zip(state.token_sha256.iter()) {
        diff |= a ^ b;
    }
    if diff != 0 {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "invalid bearer token"})),
        )
            .into_response();
    }

    state.last_activity.store(now_secs(), Ordering::Relaxed);
    let _guard = ClientGuard {
        clients: Arc::clone(&state.clients),
        last_activity: Arc::clone(&state.last_activity),
    };
    state.clients.fetch_add(1, Ordering::Relaxed);

    request.extensions_mut().insert(AuthIdentity {
        principal: state.principal.clone(),
        bootstrap: false,
    });
    next.run(request).await
}

async fn loopback_rpc(
    AxumState(state): AxumState<Arc<LoopbackHttpState>>,
    headers: HeaderMap,
    Json(req): Json<Request>,
) -> impl IntoResponse {
    let mut params = req.params;
    if let Some(obj) = params.as_object_mut() {
        let header_agent = headers
            .get("x-hivemind-agent")
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            .filter(|s| !s.is_empty() && s.len() <= 128 && !s.chars().any(char::is_control))
            .map(str::to_owned);
        let header_session = headers
            .get("x-hivemind-session")
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            .filter(|s| !s.is_empty() && s.len() <= 128 && !s.chars().any(char::is_control))
            .map(str::to_owned);
        if header_agent.is_some() || header_session.is_some() {
            let actor = obj.entry("actor").or_insert_with(|| serde_json::json!({}));
            if let Some(map) = actor.as_object_mut() {
                if let Some(agent) = header_agent {
                    map.entry("agent")
                        .or_insert(serde_json::Value::String(agent));
                }
                if let Some(session) = header_session {
                    map.entry("session")
                        .or_insert(serde_json::Value::String(session));
                }
            }
        }
    }
    if req.method == "backup" {
        let resp = match params
            .get("dest")
            .and_then(|v| v.as_str())
            .map(PathBuf::from)
        {
            Some(dest) => match state.service.store().backup_segments(&dest) {
                Ok(report) => Response::ok(req.id, report),
                Err(err) => {
                    let err = anyhow::Error::from(err);
                    Response::err(req.id, classify(&err), format!("{err:#}"))
                }
            },
            None => Response::err(req.id, "invalid", "backup requires `dest` path"),
        };
        return Json(resp);
    }
    let resp =
        match state
            .service
            .dispatch_with_principal(&req.method, params, Some(&state.principal))
        {
            Ok(result) => Response {
                id: req.id,
                ok: true,
                result: Some(result),
                error: None,
            },
            Err(err) => Response::err(req.id, classify(&err), format!("{err:#}")),
        };
    Json(resp)
}
