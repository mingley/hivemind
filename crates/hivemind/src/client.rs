//! Client for the `hivemindd` Unix-socket protocol.

use std::fmt;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow};
use serde_json::Value;

use crate::protocol::{self, Hello, PROTOCOL, Request, Response};

/// Stage-aware error returned by [`Client::call_typed`].
#[derive(Debug)]
pub enum CallError {
    /// Failed before or while sending the request over the socket.
    Write(anyhow::Error),
    /// Request may have been sent; failed reading/decoding the daemon response.
    Read(anyhow::Error),
    /// Daemon handled the request and returned an application/protocol error.
    Rpc { code: String, message: String },
}

impl fmt::Display for CallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Write(err) | Self::Read(err) => write!(f, "{err:#}"),
            Self::Rpc { code, message } => write!(f, "{code}: {message}"),
        }
    }
}

impl std::error::Error for CallError {}

#[derive(Debug)]
pub struct Client {
    io: BufReader<UnixStream>,
    next_id: u64,
    pub hello: Hello,
}

impl Client {
    /// Connect to an existing daemon socket.
    pub fn connect(store_dir: &Path) -> Result<Self> {
        let sock = protocol::socket_path(store_dir);
        let stream =
            UnixStream::connect(&sock).with_context(|| format!("connect {}", sock.display()))?;
        stream.set_read_timeout(Some(Duration::from_secs(30)))?;
        stream.set_write_timeout(Some(Duration::from_secs(30)))?;
        let mut client = Self {
            io: BufReader::new(stream),
            next_id: 1,
            hello: Hello {
                protocol: String::new(),
                version: String::new(),
                store_id: String::new(),
                next_offset: 0,
            },
        };
        let hello: Hello = serde_json::from_value(client.call("hello", json_empty())?)?;
        if hello.protocol != PROTOCOL {
            return Err(anyhow!(
                "daemon speaks {}, this client speaks {PROTOCOL}",
                hello.protocol
            ));
        }
        client.hello = hello;
        Ok(client)
    }

    /// Connect, starting `hivemindd` if needed (sccache-style).
    pub fn connect_or_spawn(store_dir: &Path, daemon_bin: &Path) -> Result<Self> {
        if let Ok(client) = Self::connect(store_dir) {
            return Ok(client);
        }
        spawn_daemon(store_dir, daemon_bin)?;
        // Large stores may spend tens of seconds validating/replaying before
        // binding their socket. Keep retrying through that supported cold start.
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            match Self::connect(store_dir) {
                Ok(client) => return Ok(client),
                Err(err) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(25));
                    let _ = err;
                }
                Err(err) => return Err(err).context("daemon did not become ready"),
            }
        }
    }

    pub fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        self.call_typed(method, params).map_err(Into::into)
    }

    /// Send one request and classify transport vs RPC failures.
    pub fn call_typed(
        &mut self,
        method: &str,
        params: Value,
    ) -> std::result::Result<Value, CallError> {
        let id = self.next_id;
        self.next_id += 1;
        let req = Request {
            id,
            method: method.to_owned(),
            params,
        };
        let mut line = serde_json::to_string(&req).map_err(|e| CallError::Write(e.into()))?;
        line.push('\n');
        self.io
            .get_mut()
            .write_all(line.as_bytes())
            .context("write request")
            .map_err(CallError::Write)?;
        self.io
            .get_mut()
            .flush()
            .context("flush request")
            .map_err(CallError::Write)?;
        let mut response_line = String::new();
        self.io
            .read_line(&mut response_line)
            .context("read response")
            .map_err(CallError::Read)?;
        if response_line.is_empty() {
            return Err(CallError::Read(anyhow!("daemon closed the connection")));
        }
        let resp: Response =
            serde_json::from_str(&response_line).map_err(|e| CallError::Read(e.into()))?;
        if resp.id != id {
            return Err(CallError::Read(anyhow!(
                "response id {} does not match request {id}",
                resp.id
            )));
        }
        if resp.ok {
            Ok(resp.result.unwrap_or(Value::Null))
        } else {
            let err = resp.error.unwrap_or(protocol::ErrorBody {
                code: "error".into(),
                message: "unknown error".into(),
            });
            Err(CallError::Rpc {
                code: err.code,
                message: err.message,
            })
        }
    }
}

fn json_empty() -> Value {
    Value::Object(serde_json::Map::new())
}

fn spawn_daemon(store_dir: &Path, daemon_bin: &Path) -> Result<()> {
    let mut cmd = Command::new(daemon_bin);
    cmd.arg("--dir")
        .arg(store_dir)
        .arg("--detached")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // Put hivemindd in its own process group so signals sent to an exiting MCP
    // client's process group never kill the shared daemon for other sessions.
    #[cfg(unix)]
    cmd.process_group(0);
    let mut child = cmd
        .spawn()
        .with_context(|| format!("spawn {}", daemon_bin.display()))?;
    // Reap failed startup races and eventual idle exits while the client stays
    // alive. The daemon itself runs in the foreground with redirected stdio.
    thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Resolve the `hivemindd` binary next to the current executable when possible.
pub fn default_daemon_bin() -> PathBuf {
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let sibling = dir.join("hivemindd");
        if sibling.exists() {
            return sibling;
        }
    }
    PathBuf::from("hivemindd")
}
