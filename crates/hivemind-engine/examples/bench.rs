//! Quick engine measurements against the provisional targets in
//! `docs/01-problem-and-goals.md`: ingest rate, replay time, resident memory,
//! recall latency, and fsync-acknowledged write latency.
//!
//! Run with: `cargo run --release -p hivemind-engine --example bench -- 100000`

use std::time::{Duration, Instant};

use hivemind_core::{Actor, ClaimData, ClaimKind, Scope};
use hivemind_engine::{Durability, Options, Query, Remember, Store};

/// xorshift64*: deterministic data without extra dependencies.
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

fn claim(rng: &mut Rng, scope: &Scope) -> (ClaimData, String) {
    let (a, b, c) = (word(rng), word(rng), word(rng));
    let code = 100 + rng.below(9_000);
    let os = rng.pick(OS);
    let reason = rng.pick(REASONS);
    let (kind, statement, query) = match rng.below(4) {
        0 => (
            ClaimKind::Solution,
            format!(
                "cargo build fails with error[E{code}] in {a}::{b} on {os} because {reason}; fix by enabling the {c} feature"
            ),
            format!("error[E{code}] {a}::{b}"),
        ),
        1 => (
            ClaimKind::Warning,
            format!("test_{a}_{b} is flaky on {os}: {reason}"),
            format!("test_{a}_{b} flaky"),
        ),
        2 => (
            ClaimKind::Fact,
            format!(
                "{a}-{b} {}.{}.{} requires {c} >= 2 and breaks the {a}Handler trait bound",
                rng.below(3),
                rng.below(20),
                rng.below(200)
            ),
            format!("{a}-{b} {a}Handler"),
        ),
        _ => (
            ClaimKind::Decision,
            format!("Use {a} for {b} storage; {c} was rejected because {reason}"),
            format!("why {a} for {b}"),
        ),
    };
    let mut data = ClaimData::new(kind, statement, scope.clone());
    data.about.push(format!("pkg:cargo/{a}-{b}"));
    data.applies_when.insert("os".into(), os.into());
    (data, query)
}

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    sorted[((sorted.len() as f64 - 1.0) * p).round() as usize]
}

fn rss_mb() -> f64 {
    let pid = std::process::id().to_string();
    std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse::<f64>().ok())
        .map_or(f64::NAN, |kb| kb / 1024.0)
}

fn report(label: &str, latencies: &mut [Duration]) {
    latencies.sort();
    println!(
        "{label} ({} samples): p50 {:?}, p95 {:?}, p99 {:?}",
        latencies.len(),
        percentile(latencies, 0.5),
        percentile(latencies, 0.95),
        percentile(latencies, 0.99)
    );
}

fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(100_000);
    let dir = tempfile::tempdir().expect("temp dir");
    let actor = Actor::agent("bench");
    let scopes: Vec<Scope> = (0..20)
        .map(|i| format!("project:repo-{i}").parse().unwrap())
        .collect();
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut queries = Vec::new();

    println!("hivemind engine bench: {n} claims across 20 project scopes");
    println!("rss at start: {:.1} MB", rss_mb());

    let no_fsync = Options {
        durability: Durability::NoFsync,
        ..Options::default()
    };
    let start = Instant::now();
    {
        let store = Store::open(dir.path(), no_fsync).unwrap();
        for i in 0..n {
            let (data, query) = claim(&mut rng, &scopes[i % scopes.len()]);
            if i % (n / 200).max(1) == 0 {
                queries.push((query, scopes[i % scopes.len()].clone()));
            }
            store.remember(&actor, Remember::new(data)).unwrap();
        }
        let stats = store.stats();
        println!(
            "ingest without fsync: {:.0} claims/s; log {:.1} MB in {} segments; {} terms",
            n as f64 / start.elapsed().as_secs_f64(),
            stats.log_bytes as f64 / 1e6,
            stats.log_segments,
            stats.terms
        );
    }

    let start = Instant::now();
    let store = Store::open(dir.path(), Options::default()).unwrap();
    println!(
        "replay on open: {:.2} s; rss after replay: {:.1} MB",
        start.elapsed().as_secs_f64(),
        rss_mb()
    );

    let mut scoped = Vec::new();
    let mut hits = 0;
    for (text, scope) in &queries {
        let query = Query {
            text: text.clone(),
            scopes: vec![scope.clone()],
            ..Query::default()
        };
        let start = Instant::now();
        let result = store.recall(&query);
        scoped.push(start.elapsed());
        hits += usize::from(!result.cards.is_empty());
    }
    report("recall within one scope", &mut scoped);
    println!("queries with at least one card: {hits}/{}", queries.len());

    let mut global = Vec::new();
    for (text, _) in queries.iter().take(100) {
        let query = Query {
            text: text.clone(),
            ..Query::default()
        };
        let start = Instant::now();
        store.recall(&query);
        global.push(start.elapsed());
    }
    report("recall across all scopes", &mut global);

    let mut writes = Vec::new();
    for i in 0..200 {
        let (data, _) = claim(&mut rng, &scopes[i % scopes.len()]);
        let start = Instant::now();
        store.remember(&actor, Remember::new(data)).unwrap();
        writes.push(start.elapsed());
    }
    report("write acknowledged after fsync", &mut writes);
    println!("rss at end: {:.1} MB", rss_mb());
}
