//! `hivemindd`: owns a store and serves the Unix-socket protocol.

use std::fs;
use std::io::ErrorKind;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, anyhow};
use clap::Parser;
use hivemind_engine::{Durability, Options, Store};
use tracing::{info, warn};

use crate::embedder::EmbedderArgs;
use crate::protocol::{self, RUN_DIR, Request, Response};
use crate::service::Service;

const CLIENT_POLL_READ_TIMEOUT: Duration = Duration::from_secs(1);
const PARTIAL_FRAME_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Parser, Debug)]
#[command(name = "hivemindd", version, about = "hivemind store daemon")]
pub struct DaemonArgs {
    /// Store directory [default: ~/.hivemind/store].
    #[arg(long, env = "HIVEMIND_DIR")]
    pub dir: Option<PathBuf>,
    /// Seconds with no clients before exit. 0 disables idle exit.
    #[arg(long, env = "HIVEMIND_IDLE_TIMEOUT", default_value_t = 600)]
    pub idle_timeout: u64,
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

    info!(
        store = %dir.display(),
        socket = %sock_path.display(),
        idle_timeout = args.idle_timeout,
        detached = args.detached,
        "hivemindd ready"
    );

    let last_activity = Arc::new(AtomicU64::new(now_secs()));
    let clients = Arc::new(AtomicU64::new(0));
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

    // Give active handlers a brief window to finish in-flight responses cleanly.
    let drain_deadline = Instant::now() + Duration::from_secs(2);
    while clients.load(Ordering::Relaxed) > 0 && Instant::now() < drain_deadline {
        thread::sleep(Duration::from_millis(20));
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
