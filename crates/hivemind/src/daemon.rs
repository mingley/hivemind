//! `hivemindd`: owns a store and serves the Unix-socket protocol.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, anyhow};
use clap::Parser;
use hivemind_engine::{Durability, Options, Store};
use tracing::{info, warn};

use crate::protocol::{self, RUN_DIR, Request, Response};
use crate::service::Service;

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
    let service = Arc::new(Service::new(store));
    let run_dir = dir.join(RUN_DIR);
    fs::create_dir_all(&run_dir)?;
    let mut perms = fs::metadata(&run_dir)?.permissions();
    perms.set_mode(0o700);
    fs::set_permissions(&run_dir, perms)?;

    let sock_path = protocol::socket_path(&dir);
    if sock_path.exists() {
        // Stale socket from a previous crash: only remove if we hold the store lock.
        let _ = fs::remove_file(&sock_path);
    }
    let listener =
        UnixListener::bind(&sock_path).with_context(|| format!("bind {}", sock_path.display()))?;
    fs::set_permissions(&sock_path, fs::Permissions::from_mode(0o600))?;
    // Ensure the socket file is removed when we exit.
    let _guard = SocketGuard(sock_path.clone());

    info!(
        store = %dir.display(),
        socket = %sock_path.display(),
        idle_timeout = args.idle_timeout,
        "hivemindd ready"
    );

    let last_activity = Arc::new(AtomicU64::new(now_secs()));
    let clients = Arc::new(AtomicU64::new(0));
    listener.set_nonblocking(true)?;

    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                last_activity.store(now_secs(), Ordering::Relaxed);
                clients.fetch_add(1, Ordering::Relaxed);
                let service = Arc::clone(&service);
                let last_activity = Arc::clone(&last_activity);
                let clients = Arc::clone(&clients);
                thread::spawn(move || {
                    if let Err(err) = handle_client(stream, &service) {
                        warn!("client error: {err:#}");
                    }
                    clients.fetch_sub(1, Ordering::Relaxed);
                    last_activity.store(now_secs(), Ordering::Relaxed);
                });
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
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
    Ok(())
}

fn handle_client(stream: UnixStream, service: &Service) -> Result<()> {
    use std::io::{BufRead, BufReader};
    stream.set_read_timeout(Some(Duration::from_secs(300)))?;
    stream.set_write_timeout(Some(Duration::from_secs(30)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut writer = stream;
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            break;
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
            let resp = Response::ok(req.id, serde_json::json!({"stopping": true}));
            write_response(&mut writer, &resp)?;
            // Best-effort: process exit is handled by idle path; tests can kill.
            break;
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
