//! Post-mitigation RSS harness for the ≤50 MB laptop target.
//!
//! Methodology:
//! 1. Release build, no embedding model.
//! 2. Ingest N synthetic claims (`Durability::NoFsync`) across 20 scopes with
//!    the same generator seed as `examples/bench` (`0x9E3779B97F4A7C15`).
//! 3. Drop the writer store, reopen with default `Options` (full replay).
//! 4. Read RSS via `ps -o rss=` for this PID (KiB → MiB).
//! 5. Optional `--seal` calls `Store::seal_lexical` (writes an mmap segment;
//!    today's `LexicalSegment::open` copies into an anonymous map and does
//!    **not** free the live BM25 heap, so expect RSS to stay flat or rise).
//!
//! Run:
//! ```text
//! cargo run --release -p hivemind-engine --example rss_harness -- 100000
//! cargo run --release -p hivemind-engine --example rss_harness -- 10000
//! cargo run --release -p hivemind-engine --example rss_harness -- 100000 --seal
//! ```

use std::process::Command;
use std::time::Instant;

use hivemind_core::{Actor, ClaimData, ClaimKind, Scope};
use hivemind_engine::{Durability, Options, Remember, Store};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len())]
    }
}

const SYLLABLES: &[&str] = &[
    "ka", "zo", "mi", "tor", "lex", "vin", "qua", "rel", "dax", "pon", "sur", "fen", "gil", "hob",
    "jut", "nim", "oct", "pry", "wex", "yal",
];
const OS: &[&str] = &["macos", "linux", "windows"];
const REASONS: &[&str] = &[
    "the lockfile pins an older transitive version",
    "TZ is not UTC on CI runners",
    "the linker cannot find the system library",
    "a race between the writer and the flush thread",
    "the feature flag changes the default allocator",
];

fn word(rng: &mut Rng) -> String {
    (0..2 + rng.below(2)).map(|_| rng.pick(SYLLABLES)).collect()
}

fn claim(rng: &mut Rng, scope: &Scope) -> ClaimData {
    let (a, b, c) = (word(rng), word(rng), word(rng));
    let code = 100 + rng.below(9_000);
    let os = rng.pick(OS);
    let reason = rng.pick(REASONS);
    let (kind, statement) = match rng.below(4) {
        0 => (
            ClaimKind::Solution,
            format!(
                "cargo build fails with error[E{code}] in {a}::{b} on {os} because {reason}; fix by enabling the {c} feature"
            ),
        ),
        1 => (
            ClaimKind::Warning,
            format!("test_{a}_{b} is flaky on {os}: {reason}"),
        ),
        2 => (
            ClaimKind::Fact,
            format!(
                "{a}-{b} {}.{}.{} requires {c} >= 2 and breaks the {a}Handler trait bound",
                rng.below(3),
                rng.below(20),
                rng.below(200)
            ),
        ),
        _ => (
            ClaimKind::Decision,
            format!("Use {a} for {b} storage; {c} was rejected because {reason}"),
        ),
    };
    let mut data = ClaimData::new(kind, statement, scope.clone());
    data.about.push(format!("pkg:cargo/{a}-{b}"));
    data.applies_when.insert("os".into(), os.into());
    data
}

fn rss_mb() -> f64 {
    let pid = std::process::id().to_string();
    Command::new("ps")
        .args(["-o", "rss=", "-p", &pid])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse::<f64>().ok())
        .map_or(f64::NAN, |kb| kb / 1024.0)
}

fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(100_000);
    let seal = std::env::args().any(|a| a == "--seal");
    let dir = tempfile::tempdir().expect("temp dir");
    let actor = Actor::agent("rss-harness");
    let scopes: Vec<Scope> = (0..20)
        .map(|i| format!("project:repo-{i}").parse().unwrap())
        .collect();
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);

    println!("hivemind RSS harness");
    println!(
        "host: {} / {}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    println!("claims: {n}; seed: 0x9E3779B97F4A7C15; scopes: 20; embedder: none");
    println!("rss metric: ps -o rss= (KiB) / 1024 → MiB, same PID as store");
    println!("rss at start: {:.1} MB", rss_mb());

    let no_fsync = Options {
        durability: Durability::NoFsync,
        ..Options::default()
    };
    let start = Instant::now();
    {
        let store = Store::open(dir.path(), no_fsync).unwrap();
        for i in 0..n {
            store
                .remember(
                    &actor,
                    Remember::new(claim(&mut rng, &scopes[i % scopes.len()])),
                )
                .unwrap();
        }
        let stats = store.stats();
        println!(
            "ingest: {:.0} claims/s; log {:.1} MB; terms {}; interned {}",
            n as f64 / start.elapsed().as_secs_f64(),
            stats.log_bytes as f64 / 1e6,
            stats.terms,
            stats.interned_strings
        );
    }

    let start = Instant::now();
    let store = Store::open(dir.path(), Options::default()).unwrap();
    let stats = store.stats();
    let after_replay = rss_mb();
    println!(
        "replay on open: {:.2} s; rss after replay: {:.1} MB; claims {}; terms {}; interned {}",
        start.elapsed().as_secs_f64(),
        after_replay,
        stats.claims,
        stats.terms,
        stats.interned_strings
    );

    if seal {
        let before = rss_mb();
        let path = store.seal_lexical().unwrap();
        println!(
            "after seal_lexical (file written; live BM25 still held): {:.1} MB → {:.1} MB; {}",
            before,
            rss_mb(),
            path.display()
        );
    }

    println!("target (docs/01, 100k idle, no embedder): ≤ 50 MB");
    println!(
        "verdict: {}",
        if after_replay <= 50.0 {
            "PASS"
        } else {
            "MISS — residual dominated by in-memory BM25 dictionary/postings + keydir rows"
        }
    );
}
