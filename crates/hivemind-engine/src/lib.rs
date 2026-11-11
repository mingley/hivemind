//! hivemind's storage engine.
//!
//! - **Log** ([`log`]): an append-only, checksummed, segmented log. It is the
//!   source of truth; each append is one record, flushed before it is
//!   acknowledged.
//! - **Blobs** ([`blob`]): content-addressed artifacts, durable before they
//!   are referenced.
//! - **Projections** ([`state`]): claims, entities, edges, artifacts, and a
//!   code-aware BM25 index ([`lexical`]), rebuilt from the log on open.
//! - **Retrieval** ([`recall`]): filters, ranking, and compact cards fitted to
//!   a token budget.
//! - **[`Store`]**: the single-owner API tying these together.
//!
//! Design and durability contract: `docs/04-architecture-and-decisions.md`.

pub mod blob;
pub mod codec;
pub mod error;
mod fsutil;
pub mod keys;
pub mod lexical;
pub mod log;
pub mod recall;
pub mod state;
pub mod store;
pub mod vector;

pub use error::{Error, Result};
pub use log::{Durability, Recovery};
pub use recall::{Card, ClaimDetail, Flag, Query, Recall, render_card};
pub use state::{ArtifactRow, Belief, LeaseRow, STATEMENT_CACHE_CHARS};
pub use store::{
    ChangesFilter, LeaseReceipt, LeaseView, Manifest, Options, Receipt, Remember, Stats, Store,
    VerifyReport,
};
pub use vector::{Embedder, EmbeddingSpace, HashEmbedder, Quantization, VectorIndex, rrf_fuse};
