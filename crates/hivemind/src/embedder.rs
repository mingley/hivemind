//! Optional embedder wiring for CLI / daemon / serve.
//!
//! Lean default is lexical-only ([`EmbedderChoice::None`]). Operators opt in
//! with `--embedder` / `HIVEMIND_EMBEDDER` (see `docs/operations.md`).

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use clap::Args;
use hivemind_engine::{EmbedderChoice, Store, build_embedder};

/// Shared flags for selecting an optional embedder.
#[derive(Clone, Debug, Args)]
pub struct EmbedderArgs {
    /// Optional embedder: `none` (default, lexical-only), `hash` (tests), or
    /// `model2vec` (requires `--features embeddings` and `--embedder-model`).
    #[arg(long, env = "HIVEMIND_EMBEDDER", default_value = "none")]
    pub embedder: String,
    /// Model directory (or Hub id with `embeddings-hf`) for `model2vec`.
    #[arg(long, env = "HIVEMIND_EMBEDDER_MODEL")]
    pub embedder_model: Option<PathBuf>,
    /// Dimensions for the `hash` embedder [default: 384].
    #[arg(long, env = "HIVEMIND_EMBEDDER_DIMS")]
    pub embedder_dims: Option<u32>,
    /// Optional embedding-space revision label (recorded on the space).
    #[arg(long, env = "HIVEMIND_EMBEDDER_REVISION")]
    pub embedder_revision: Option<String>,
}

impl Default for EmbedderArgs {
    fn default() -> Self {
        Self {
            embedder: "none".into(),
            embedder_model: None,
            embedder_dims: None,
            embedder_revision: None,
        }
    }
}

impl EmbedderArgs {
    pub fn choice(&self) -> Result<EmbedderChoice> {
        Ok(EmbedderChoice::parse_kind(&self.embedder)
            .map_err(|e| anyhow!(e))?
            .with_model(self.embedder_model.clone())
            .with_dims(self.embedder_dims)
            .with_revision(self.embedder_revision.clone()))
    }

    /// Attach the configured embedder to an open store (no-op for `none`).
    pub fn apply(&self, store: &Store) -> Result<()> {
        let choice = self.choice()?;
        if let Some(embedder) = build_embedder(&choice).map_err(|e| anyhow!(e))? {
            store
                .set_embedder(Arc::clone(&embedder))
                .map_err(|e| anyhow!(e))?;
        }
        Ok(())
    }
}
