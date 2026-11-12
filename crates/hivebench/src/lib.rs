//! hivebench: offline retrieval evaluation and scenario tooling for hivemind.
//!
//! See `docs/m2-implementation-notes.md`, `docs/hivebench-ledger.md`, and
//! `docs/05-evaluation-plan.md`.

pub mod baselines;
pub mod er;
pub mod harness;
pub mod ledger;
pub mod scenario;
pub mod tools;

pub use er::{ErMetrics, run_er};
pub use harness::{LoopOptions, RunRecord, dry_run, run_controlled};
pub use ledger::{ActorKind, Category, CountSource, Ledger, LedgerEntry, estimate_tokens};
pub use scenario::{Probe, Scenario, load_scenario, load_scenarios};
