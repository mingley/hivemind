//! End-to-end CLI behavior.

use std::path::Path;
use std::process::Command;

fn hivemind(dir: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_hivemind"));
    cmd.arg("--dir")
        .arg(dir)
        .arg("--as")
        .arg("cli-test")
        .arg("--no-fsync");
    cmd
}

fn run(cmd: &mut Command) -> String {
    let output = cmd.output().unwrap();
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn json(cmd: &mut Command) -> serde_json::Value {
    serde_json::from_str(&run(cmd.arg("--json"))).unwrap()
}

#[test]
fn remember_recall_show_and_supersede() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let log = dir.join("build.log");
    std::fs::write(
        &log,
        "ld: symbol(s) not found for architecture arm64\n  _SSL_CTX_new",
    )
    .unwrap();

    let first = json(hivemind(dir).args([
        "remember",
        "Linking fails with _SSL_CTX_new not found on macOS arm64; set OPENSSL_DIR to brew openssl@3",
        "--kind", "solution",
        "--scope", "project:acme/api",
        "--about", "pkg:cargo/openssl-sys",
        "--when", "os=macos",
        "--when", "dep:cargo/openssl-sys=>=0.9.100, <0.10",
        "--field", "fix=export OPENSSL_DIR=$(brew --prefix openssl@3)",
        "--cite", ".cargo/config.toml:3-5@4b2c9e1",
        "--evidence-file",
    ]).arg(&log));
    let id = first["claim"].as_str().unwrap().to_owned();

    let hits = json(hivemind(dir).args([
        "recall",
        "_SSL_CTX_new",
        "not",
        "found",
        "--ctx",
        "os=macos",
        "--ctx",
        "dep:cargo/openssl-sys=0.9.103",
    ]));
    assert_eq!(hits["cards"][0]["id"], id.as_str());
    assert_eq!(hits["cards"][0]["applicability"]["state"], "match");

    let none = json(hivemind(dir).args(["recall", "_SSL_CTX_new", "--ctx", "os=linux"]));
    assert_eq!(none["cards"].as_array().unwrap().len(), 0);

    let prefix = &id["clm_".len()..id.len() - 8];
    let detail = run(hivemind(dir).args(["show", prefix, "--evidence"]));
    assert!(detail.contains("OPENSSL_DIR"));
    assert!(detail.contains("| ld: symbol(s) not found"));

    let second = json(hivemind(dir).args([
        "remember",
        "Linking fails with _SSL_CTX_new on macOS arm64; install openssl@3 with Homebrew and set OPENSSL_DIR",
        "--kind", "solution",
        "--scope", "project:acme/api",
        "--supersedes", prefix,
    ]));
    let current = json(hivemind(dir).args(["recall", "_SSL_CTX_new"]));
    assert_eq!(current["cards"].as_array().unwrap().len(), 1);
    assert_eq!(current["cards"][0]["id"], second["claim"]);

    let history = json(hivemind(dir).args(["recall", "_SSL_CTX_new", "--history"]));
    assert_eq!(history["cards"].as_array().unwrap().len(), 2);

    let text = run(hivemind(dir).args(["recall", "_SSL_CTX_new"]));
    assert!(text.contains("| solution | active |"));
    assert!(text.contains("1 of 1 shown"));

    let stats = json(hivemind(dir).arg("stats"));
    assert_eq!(stats["claims"], 2);
    assert_eq!(stats["by_status"]["superseded"], 1);
    run(hivemind(dir).arg("verify"));
}

#[test]
fn rejects_secrets_and_bad_input_with_clear_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let output = hivemind(tmp.path())
        .args([
            "remember",
            "use token ghp_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa for the API",
            "--scope",
            "project:x",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("likely secret"));

    let output = hivemind(tmp.path())
        .args(["remember", "x", "--scope", "planet:mars"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid scope"));

    let output = hivemind(tmp.path())
        .args([
            "remember",
            "Cache warmup takes 5 minutes",
            "--scope",
            "project:a",
            "--review-by",
            "9000y",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success(), "far-future times are rejected");
}

#[test]
fn prerelease_toolchains_still_match() {
    let tmp = tempfile::tempdir().unwrap();
    run(hivemind(tmp.path()).args([
        "remember",
        "rustdoc --check-cfg needs rust 1.65 or newer",
        "--scope",
        "project:a",
        "--when",
        "toolchain:rust=>=1.65",
    ]));
    let hits = json(hivemind(tmp.path()).args([
        "recall",
        "check-cfg",
        "--ctx",
        "toolchain:rust=1.80.0-nightly",
    ]));
    assert_eq!(hits["cards"].as_array().unwrap().len(), 1);
    assert_eq!(hits["cards"][0]["applicability"]["state"], "match");
}

#[test]
fn torn_writes_are_reported_on_open() {
    let tmp = tempfile::tempdir().unwrap();
    run(hivemind(tmp.path()).args(["remember", "first claim", "--scope", "project:a"]));
    let segment = tmp.path().join("log").join("00000000000000000000.hlog");
    let mut bytes = std::fs::read(&segment).unwrap();
    bytes.extend_from_slice(&[0x5a; 21]);
    std::fs::write(&segment, bytes).unwrap();
    let output = hivemind(tmp.path())
        .args(["recall", "first"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("removed 21 bytes"));
    let stats = json(hivemind(tmp.path()).arg("stats"));
    assert_eq!(stats["recoveries"].as_array().unwrap().len(), 1);
}
