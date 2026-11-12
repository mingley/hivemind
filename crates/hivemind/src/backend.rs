//! CLI backend: talk to `hivemindd` when it is running, otherwise open the store.

use std::path::Path;

use anyhow::Result;
use serde_json::Value;

use crate::client::Client;
use crate::embedder::EmbedderArgs;
use hivemind_engine::{Durability, Options, Store};

#[derive(Debug)]
pub enum Backend {
    Daemon(Client),
    Local(Box<Store>),
}

impl Backend {
    /// Prefer a live daemon; fall back to opening the store directly.
    pub fn open(dir: &Path, no_fsync: bool) -> Result<Self> {
        Self::open_with_embedder(dir, no_fsync, &EmbedderArgs::default())
    }

    /// Like [`Self::open`], then attach an optional embedder on the local path.
    pub fn open_with_embedder(dir: &Path, no_fsync: bool, embedder: &EmbedderArgs) -> Result<Self> {
        if let Ok(client) = Client::connect(dir) {
            return Ok(Self::Daemon(client));
        }
        let options = Options {
            durability: if no_fsync {
                Durability::NoFsync
            } else {
                Durability::Fsync
            },
            ..Options::default()
        }
        .with_env_overrides()?;
        let store = Store::open(dir, options)?;
        embedder.apply(&store)?;
        Ok(Self::Local(Box::new(store)))
    }

    pub fn store(&self) -> Option<&Store> {
        match self {
            Self::Local(store) => Some(store),
            Self::Daemon(_) => None,
        }
    }

    pub fn store_mut(&mut self) -> Option<&mut Store> {
        match self {
            Self::Local(store) => Some(store),
            Self::Daemon(_) => None,
        }
    }

    pub fn call_daemon(&mut self, method: &str, params: Value) -> Result<Option<Value>> {
        match self {
            Self::Daemon(client) => Ok(Some(client.call(method, params)?)),
            Self::Local(_) => Ok(None),
        }
    }

    pub fn is_daemon(&self) -> bool {
        matches!(self, Self::Daemon(_))
    }
}
