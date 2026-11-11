//! Crash safety: every write the store acknowledged must survive `kill -9`,
//! including kills that land in the middle of the next write.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

fn hivemind(dir: &std::path::Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_hivemind"));
    cmd.arg("--dir").arg(dir).arg("--as").arg("crash-test");
    cmd
}

#[test]
fn acknowledged_writes_survive_kill_9() {
    let tmp = tempfile::tempdir().unwrap();
    let mut acked: BTreeMap<u64, String> = BTreeMap::new();

    for round in 0..12u64 {
        let mut child = hivemind(tmp.path())
            .arg("debug-write-loop")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let mut lines = BufReader::new(stdout).lines();
        let target = 3 + (round * 7) % 25;
        for _ in 0..target {
            let line = lines
                .next()
                .expect("writer exited before acknowledging")
                .unwrap();
            let mut parts = line.split_whitespace();
            assert_eq!(parts.next(), Some("ack"), "{line}");
            let offset: u64 = parts.next().unwrap().parse().unwrap();
            let claim = parts.next().unwrap().to_owned();
            assert!(
                acked.insert(offset, claim).is_none(),
                "offset {offset} reused"
            );
        }
        // SIGKILL, usually while the next write is in flight.
        child.kill().unwrap();
        child.wait().unwrap();
    }

    let log = hivemind(tmp.path())
        .args(["log", "--limit", "1000000"])
        .output()
        .unwrap();
    assert!(
        log.status.success(),
        "{}",
        String::from_utf8_lossy(&log.stderr)
    );
    let mut survived: BTreeMap<u64, String> = BTreeMap::new();
    for line in String::from_utf8(log.stdout).unwrap().lines() {
        let entry: serde_json::Value = serde_json::from_str(line).unwrap();
        let offset = entry["offset"].as_u64().unwrap();
        let claim = entry["event"]["body"]["claim"].as_str().unwrap().to_owned();
        survived.insert(offset, claim);
    }
    for (offset, claim) in &acked {
        assert_eq!(
            survived.get(offset),
            Some(claim),
            "acknowledged write at offset {offset} was lost or replaced"
        );
    }
    let offsets: Vec<u64> = survived.keys().copied().collect();
    assert_eq!(
        offsets,
        (0..offsets.len() as u64).collect::<Vec<_>>(),
        "offsets must be contiguous"
    );

    let verify = hivemind(tmp.path())
        .args(["--json", "verify"])
        .output()
        .unwrap();
    assert!(
        verify.status.success(),
        "{}",
        String::from_utf8_lossy(&verify.stdout)
    );
}
