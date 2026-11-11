//! Client for the `hivemindd` Unix-socket protocol.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow};
use serde_json::Value;

use crate::protocol::{self, Hello, PROTOCOL, Request, Response};

#[derive(Debug)]
pub struct Client {
    stream: UnixStream,
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
            stream,
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
        let deadline = Instant::now() + Duration::from_secs(5);
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
        let id = self.next_id;
        self.next_id += 1;
        let req = Request {
            id,
            method: method.to_owned(),
            params,
        };
        let mut line = serde_json::to_string(&req)?;
        line.push('\n');
        self.stream
            .write_all(line.as_bytes())
            .context("write request")?;
        self.stream.flush()?;
        let mut reader = BufReader::new(self.stream.try_clone()?);
        let mut response_line = String::new();
        reader
            .read_line(&mut response_line)
            .context("read response")?;
        if response_line.is_empty() {
            return Err(anyhow!("daemon closed the connection"));
        }
        let resp: Response = serde_json::from_str(&response_line)?;
        if resp.id != id {
            return Err(anyhow!(
                "response id {} does not match request {id}",
                resp.id
            ));
        }
        if resp.ok {
            Ok(resp.result.unwrap_or(Value::Null))
        } else {
            let err = resp.error.unwrap_or(protocol::ErrorBody {
                code: "error".into(),
                message: "unknown error".into(),
            });
            Err(anyhow!("{}: {}", err.code, err.message))
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
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // Detach: the daemon double-forks itself; here we just start it.
    cmd.spawn()
        .with_context(|| format!("spawn {}", daemon_bin.display()))?;
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
