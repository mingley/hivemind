//! M6: deterministic simulation — seeded ops with crash/reopen checking
//! that acknowledged writes survive and projections match a full replay.

use std::collections::BTreeSet;

use hivemind_core::{Actor, ClaimData, ClaimKind, Scope};
use hivemind_engine::{Durability, Options, Query, Remember, Store};

/// Tiny deterministic LCG so the suite needs no extra RNG crate.
struct Lcg(u64);

impl Lcg {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1);
        self.0
    }

    fn next_usize(&mut self, n: usize) -> usize {
        if n == 0 {
            return 0;
        }
        (self.next_u64() as usize) % n
    }
}

fn options() -> Options {
    Options {
        durability: Durability::NoFsync,
        ..Options::default()
    }
}

fn scope() -> Scope {
    "project:acme/sim".parse().unwrap()
}

fn actor() -> Actor {
    Actor::agent("sim")
}

#[test]
fn deterministic_ops_survive_reopen_and_match_replay() {
    let tmp = tempfile::tempdir().unwrap();
    let mut rng = Lcg(0x46e6_5151);
    let mut acked = BTreeSet::new();

    {
        let store = Store::open(tmp.path(), options()).unwrap();
        for step in 0..40u64 {
            match rng.next_usize(4) {
                0 | 1 => {
                    let statement = format!(
                        "sim claim step={step} token={}",
                        rng.next_u64() % 10_000
                    );
                    let data = ClaimData::new(ClaimKind::Fact, statement, scope());
                    let receipt = store.remember(&actor(), Remember::new(data)).unwrap();
                    acked.insert(receipt.offset);
                    if let Some(id) = receipt.claim {
                        let _ = store.get(&id, None);
                    }
                }
                2 => {
                    let _ = store.recall(&Query {
                        text: format!("sim claim step={}", rng.next_usize(40)),
                        ..Query::default()
                    });
                }
                _ => {
                    if step > 0 && step.is_multiple_of(10) {
                        store.checkpoint().unwrap();
                    }
                }
            }
        }
        // Crash: drop without graceful shutdown.
    }

    let reopened = Store::open(tmp.path(), options()).unwrap();
    assert!(reopened.next_offset() > 0);
    for offset in &acked {
        assert!(
            *offset < reopened.next_offset(),
            "acked offset {offset} missing after reopen (next={})",
            reopened.next_offset()
        );
    }

    // Full replay into a fresh dir by copying log+manifest must yield the same tip.
    let mirror = tempfile::tempdir().unwrap();
    let report = reopened.backup_segments(mirror.path()).unwrap();
    assert_eq!(report.next_offset, reopened.next_offset());
    let mirrored = Store::open(mirror.path(), options()).unwrap();
    assert_eq!(mirrored.next_offset(), reopened.next_offset());
    assert_eq!(mirrored.stats().claims, reopened.stats().claims);
}
