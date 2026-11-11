//! hivebench: offline retrieval evaluation and scenario tooling for hivemind.
//!
//! See `docs/m2-implementation-notes.md` and `docs/05-evaluation-plan.md`.

pub mod baselines;
pub mod er;
pub mod harness;
pub mod ledger;
pub mod scenario;

pub use er::{ErMetrics, run_er};
pub use scenario::{Probe, Scenario, load_scenario, load_scenarios};
