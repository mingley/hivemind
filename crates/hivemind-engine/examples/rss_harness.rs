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
//! Multi-N: pass several claim counts; ingest is incremental on one store, with
//! a full reopen/replay measurement at each checkpoint (sorted ascending).
//!
//! Run:
//! ```text
//! cargo run --release -p hivemind-engine --example rss_harness -- 100000
//! cargo run --release -p hivemind-engine --example rss_harness -- 10000 100000
//! cargo run --release -p hivemind-engine --example rss_harness -- --dir /tmp/rss 10000 100000 --progress
//! cargo run --release -p hivemind-engine --example rss_harness -- 100000 --seal
//! ```

use std::path::{Path, PathBuf};
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

struct Args {
    ns: Vec<usize>,
    dir: Option<PathBuf>,
    progress: bool,
    seal: bool,
}

fn parse_args() -> Args {
    let mut ns = Vec::new();
    let mut dir = None;
    let mut progress = false;
    let mut seal = false;
    let mut raw = std::env::args().skip(1);
    while let Some(a) = raw.next() {
        match a.as_str() {
            "--dir" => {
                let path = raw.next().unwrap_or_else(|| {
                    eprintln!("rss_harness: --dir requires a path");
                    std::process::exit(2);
                });
                dir = Some(PathBuf::from(path));
            }
            "--progress" => progress = true,
            "--seal" => seal = true,
            "--help" | "-h" => {
                eprintln!(
                    "usage: rss_harness [N ...] [--dir PATH] [--progress] [--seal]\n\
                     \n\
                     Measure RSS after full replay at each claim count N.\n\
                     Multiple Ns ingest incrementally on one store (sorted ascending).\n\
                     Default N is 100000 when none given."
                );
                std::process::exit(0);
            }
            s if s.starts_with('-') => {
                eprintln!("rss_harness: unknown flag {s}");
                std::process::exit(2);
            }
            s => match s.parse::<usize>() {
                Ok(n) if n > 0 => ns.push(n),
                Ok(_) => {
                    eprintln!("rss_harness: N must be > 0");
                    std::process::exit(2);
                }
                Err(_) => {
                    eprintln!("rss_harness: expected positive integer N, got {s}");
                    std::process::exit(2);
                }
            },
        }
    }
    if ns.is_empty() {
        ns.push(100_000);
    }
    ns.sort_unstable();
    ns.dedup();
    Args {
        ns,
        dir,
        progress,
        seal,
    }
}

fn progress_step(n: usize) -> usize {
    // ~20 updates for large runs; keep small runs quiet-ish.
    let step = (n / 20).max(1_000);
    // Round to a nice 1k boundary when large.
    if step >= 10_000 {
        (step / 10_000) * 10_000
    } else if step >= 1_000 {
        (step / 1_000) * 1_000
    } else {
        step
    }
}

fn ingest_upto(
    path: &Path,
    from: usize,
    to: usize,
    actor: &Actor,
    scopes: &[Scope],
    rng: &mut Rng,
    progress: bool,
) {
    let no_fsync = Options {
        durability: Durability::NoFsync,
        ..Options::default()
    };
    let store = Store::open(path, no_fsync).unwrap();
    let start = Instant::now();
    let step = progress_step(to);
    for i in from..to {
        store
            .remember(actor, Remember::new(claim(rng, &scopes[i % scopes.len()])))
            .unwrap();
        if progress && (i + 1) % step == 0 {
            let done = i + 1;
            let rate = done.saturating_sub(from) as f64 / start.elapsed().as_secs_f64().max(1e-9);
            println!(
                "  progress: {done}/{to} ({:.1}%); {:.0} claims/s; rss {:.1} MB",
                100.0 * done as f64 / to as f64,
                rate,
                rss_mb()
            );
        }
    }
    let stats = store.stats();
    let written = to - from;
    println!(
        "ingest {from}→{to}: {:.0} claims/s; log {:.1} MB; terms {}; interned {}",
        written as f64 / start.elapsed().as_secs_f64().max(1e-9),
        stats.log_bytes as f64 / 1e6,
        stats.terms,
        stats.interned_strings
    );
}

fn measure_replay(path: &Path, n: usize, seal: bool) -> f64 {
    let start = Instant::now();
    let store = Store::open(path, Options::default()).unwrap();
    let stats = store.stats();
    let after_replay = rss_mb();
    println!(
        "N={n}: replay on open: {:.2} s; rss after replay: {:.1} MB; claims {}; terms {}; interned {}",
        start.elapsed().as_secs_f64(),
        after_replay,
        stats.claims,
        stats.terms,
        stats.interned_strings
    );
    println!(
        "N={n}: target (docs/01, idle, no embedder): ≤ 50 MB; verdict: {}",
        if after_replay <= 50.0 {
            "PASS"
        } else {
            "MISS — residual dominated by in-memory BM25 dictionary/postings + keydir rows"
        }
    );

    if seal {
        let before = rss_mb();
        let seal_path = store.seal_lexical().unwrap();
        println!(
            "N={n}: after seal_lexical (file written; live BM25 still held): {:.1} MB → {:.1} MB; {}",
            before,
            rss_mb(),
            seal_path.display()
        );
    }
    // Drop store before next ingest so we do not hold two projections.
    drop(store);
    after_replay
}

fn main() {
    let args = parse_args();
    let tmp = if args.dir.is_none() {
        Some(tempfile::tempdir().expect("temp dir"))
    } else {
        None
    };
    let path: PathBuf = match &args.dir {
        Some(d) => {
            std::fs::create_dir_all(d).expect("create --dir");
            d.clone()
        }
        None => tmp.as_ref().unwrap().path().to_path_buf(),
    };

    let actor = Actor::agent("rss-harness");
    let scopes: Vec<Scope> = (0..20)
        .map(|i| format!("project:repo-{i}").parse().unwrap())
        .collect();
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);

    let ns_label = args
        .ns
        .iter()
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(", ");

    println!("hivemind RSS harness");
    println!(
        "host: {} / {}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    println!(
        "claims checkpoints: [{ns_label}]; seed: 0x9E3779B97F4A7C15; scopes: 20; embedder: none"
    );
    println!("store dir: {}", path.display());
    println!("rss metric: ps -o rss= (KiB) / 1024 → MiB, same PID as store");
    println!("rss at start: {:.1} MB", rss_mb());

    // If --dir already has data, count existing claims by opening once.
    let mut have = {
        let probe = Store::open(&path, Options::default());
        match probe {
            Ok(s) => {
                let n = s.stats().claims as usize;
                // Advance RNG to stay deterministic only for fresh stores.
                // Reusing a pre-filled --dir is for resume/scale scripts; seed
                // continuity is best-effort when the dir already has claims.
                n
            }
            Err(_) => 0,
        }
    };
    if have > 0 {
        println!("existing claims in dir: {have}");
        // Burn RNG for existing claims so further ingest stays on-seed when
        // the dir was produced by this harness from empty with the same seed.
        for i in 0..have {
            let _ = claim(&mut rng, &scopes[i % scopes.len()]);
        }
    }

    let mut last_rss = f64::NAN;
    for &n in &args.ns {
        if have < n {
            println!("--- ingest to {n} ---");
            ingest_upto(&path, have, n, &actor, &scopes, &mut rng, args.progress);
            have = n;
        } else if have > n {
            eprintln!("rss_harness: dir already has {have} claims (> N={n}); measuring as-is");
        }
        println!("--- measure N={n} ---");
        last_rss = measure_replay(&path, n, args.seal);
    }

    println!(
        "done; last rss after replay: {:.1} MB; checkpoints: [{ns_label}]",
        last_rss
    );
}
