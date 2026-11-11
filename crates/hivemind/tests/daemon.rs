//! Daemon integration: two clients, remember/recall/show/changes_since, lock exclusivity.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use hivemind::client::Client;
use hivemind::protocol;
use hivemind_engine::{Durability, Error, Options, Store};
use serde_json::json;

/// Owns a `hivemindd` child and kills/waits it on drop so tests never leave orphans.
struct DaemonGuard {
    child: Child,
    dir: PathBuf,
}

impl DaemonGuard {
    fn spawn(dir: &Path, idle_timeout_secs: u64) -> Self {
        let bin = env!("CARGO_BIN_EXE_hivemindd");
        let child = Command::new(bin)
            .arg("--dir")
            .arg(dir)
            .arg("--no-fsync")
            .arg("--idle-timeout")
            .arg(idle_timeout_secs.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn hivemindd");
        let guard = Self {
            child,
            dir: dir.to_path_buf(),
        };
        guard.wait_ready(Duration::from_secs(5));
        guard
    }

    fn wait_ready(&self, timeout: Duration) {
        let sock = protocol::socket_path(&self.dir);
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if sock.exists() && Client::connect(&self.dir).is_ok() {
                return;
            }
            // Bail early if the daemon already exited.
            // `try_wait` needs &mut self; probe via a non-blocking connect loop only.
            thread::sleep(Duration::from_millis(25));
            if !is_alive(self.child.id()) {
                panic!("hivemindd exited before becoming ready");
            }
        }
        panic!("hivemindd did not become ready at {}", sock.display());
    }
}

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn is_alive(pid: u32) -> bool {
    // Signal 0 checks existence without delivering a signal.
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[test]
fn two_clients_round_trip_and_lock_stays_exclusive() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    // Short idle so a leaked daemon would still exit; Drop still kills promptly.
    let _daemon = DaemonGuard::spawn(dir, 5);

    let mut a = Client::connect(dir).expect("client a");
    let mut b = Client::connect(dir).expect("client b");
    assert_eq!(a.hello.protocol, protocol::PROTOCOL);
    assert_eq!(a.hello.store_id, b.hello.store_id);

    let remembered = a
        .call(
            "remember",
            json!({
                "statement": "Daemon clients share one store over the Unix socket",
                "kind": "fact",
                "scope": "project:acme/daemon",
            }),
        )
        .expect("remember");
    let claim = remembered["claim"].as_str().expect("claim id").to_owned();
    let offset = remembered["offset"].as_u64().expect("offset");

    let recalled = b
        .call(
            "recall",
            json!({
                "query": "Unix socket store",
                "min_offset": offset,
            }),
        )
        .expect("recall");
    let cards = recalled["cards"].as_array().expect("cards");
    assert!(
        cards.iter().any(|c| c["id"] == claim),
        "recall from second client must see the write: {recalled}"
    );

    let shown = a
        .call(
            "show",
            json!({
                "id": claim,
                "min_offset": offset,
            }),
        )
        .expect("show");
    assert_eq!(shown["id"], claim);
    assert!(
        shown["statement"]
            .as_str()
            .unwrap_or("")
            .contains("Unix socket")
    );

    let changes = b
        .call(
            "changes_since",
            json!({
                "from": 0,
                "limit": 50,
            }),
        )
        .expect("changes_since");
    let events = changes["events"].as_array().expect("events");
    assert!(
        events
            .iter()
            .any(|row| { row["event"]["body"]["claim"].as_str() == Some(claim.as_str()) }),
        "change feed must include the remember: {changes}"
    );
    assert!(changes["next_offset"].as_u64().unwrap() > offset);

    // Daemon holds the store LOCK; a second direct open must fail.
    let locked = Store::open(
        dir,
        Options {
            durability: Durability::NoFsync,
            ..Options::default()
        },
    );
    assert!(
        matches!(locked, Err(Error::Locked(_))),
        "expected Locked while daemon owns the store, got {locked:?}"
    );
}
