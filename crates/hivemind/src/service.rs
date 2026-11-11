//! Transport-independent handlers over a [`hivemind_engine::Store`].

use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow};
use hivemind_core::applicability::Context;
use hivemind_core::{
    Actor, ActorKind, ClaimData, ClaimId, ClaimKind, EdgeKind, Outcome, RetractReason, Scope,
    Timestamp,
};
use hivemind_engine::{ChangesFilter, Query, Store};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::protocol::{Hello, PROTOCOL};

#[derive(Clone, Debug, Default, Deserialize)]
pub struct ActorParams {
    pub principal: Option<String>,
    pub agent: Option<String>,
    pub session: Option<String>,
    #[serde(default)]
    pub human: bool,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct ChangesFilterParams {
    pub scopes: Option<Vec<Scope>>,
    pub claims: Option<Vec<String>>,
    pub kinds: Option<Vec<ClaimKind>>,
}

/// Shared service over an open store.
#[derive(Debug)]
pub struct Service {
    store: Store,
    version: String,
}

impl Service {
    pub fn new(store: Store) -> Self {
        Self {
            store,
            version: env!("CARGO_PKG_VERSION").to_owned(),
        }
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn hello(&self) -> Hello {
        Hello {
            protocol: PROTOCOL.to_owned(),
            version: self.version.clone(),
            store_id: self.store.manifest().store_id.clone(),
            next_offset: self.store.next_offset(),
        }
    }

    pub fn dispatch(&self, method: &str, params: Value) -> Result<Value> {
        match method {
            "hello" | "ping" => Ok(serde_json::to_value(self.hello())?),
            "remember" => self.remember(params),
            "recall" => self.recall(params),
            "show" => self.show(params),
            "retract" => self.retract(params),
            "invalidate" => self.invalidate(params),
            "link" => self.link(params),
            "feedback" => self.feedback(params),
            "changes_since" => self.changes_since(params),
            "stats" => Ok(serde_json::to_value(self.store.stats())?),
            other => Err(anyhow!("unknown method `{other}`")),
        }
    }

    fn actor_from(&self, params: &ActorParams) -> Actor {
        let principal = params
            .principal
            .clone()
            .or_else(|| std::env::var("HIVEMIND_PRINCIPAL").ok())
            .or_else(|| std::env::var("USER").ok())
            .unwrap_or_else(|| "anonymous".into());
        Actor {
            principal,
            kind: if params.human {
                ActorKind::Human
            } else {
                ActorKind::Agent
            },
            agent: params
                .agent
                .clone()
                .or_else(|| std::env::var("HIVEMIND_AGENT").ok()),
            session: params
                .session
                .clone()
                .or_else(|| std::env::var("HIVEMIND_SESSION").ok()),
        }
    }

    fn wait_if_needed(&self, min_offset: Option<u64>) -> Result<()> {
        if let Some(min) = min_offset {
            self.store
                .wait_for_offset(min, Duration::from_secs(5))
                .context("waiting for min_offset")?;
        }
        Ok(())
    }

    fn remember(&self, params: Value) -> Result<Value> {
        let p: RememberParams = serde_json::from_value(params)?;
        let actor = self.actor_from(&p.actor);
        let mut data = ClaimData::new(p.kind, p.statement, p.scope);
        data.about = p.about.unwrap_or_default();
        if let Some(when) = p.when {
            data.applies_when = when.into_iter().collect();
        }
        if let Some(fields) = p.fields {
            for (k, v) in fields {
                data.payload.insert(k, Value::String(v));
            }
        }
        let mut req = hivemind_engine::Remember::new(data);
        if let Some(ids) = p.supersedes {
            req.supersedes = resolve_many(&self.store, &ids)?;
        }
        req.idempotency_key = p.idempotency_key;
        let receipt = self.store.remember(&actor, req)?;
        Ok(serde_json::to_value(receipt)?)
    }

    fn recall(&self, params: Value) -> Result<Value> {
        let p: RecallParams = serde_json::from_value(params)?;
        self.wait_if_needed(p.min_offset)?;
        let query = Query {
            text: p.query,
            scopes: p.scopes.unwrap_or_default(),
            kinds: p.kinds.unwrap_or_default(),
            about: p.about.unwrap_or_default(),
            context: ctx_map(p.ctx.unwrap_or_default()),
            include_history: p.history.unwrap_or(false),
            include_inapplicable: p.all_contexts.unwrap_or(false),
            limit: p.limit,
            budget_tokens: p.budget,
            now: None,
            min_offset: None,
        };
        Ok(serde_json::to_value(self.store.recall(&query))?)
    }

    fn show(&self, params: Value) -> Result<Value> {
        let p: ShowParams = serde_json::from_value(params)?;
        self.wait_if_needed(p.min_offset)?;
        let id = self.store.resolve(&p.id)?;
        let ctx = p.ctx.map(ctx_map);
        let detail = self
            .store
            .get(&id, ctx.as_ref())
            .ok_or_else(|| anyhow!("claim {id} missing after resolve"))?;
        Ok(serde_json::to_value(detail)?)
    }

    fn retract(&self, params: Value) -> Result<Value> {
        let p: RetractParams = serde_json::from_value(params)?;
        let actor = self.actor_from(&p.actor);
        let id = self.store.resolve(&p.id)?;
        let corrected_by = p
            .corrected_by
            .as_deref()
            .map(|t| self.store.resolve(t))
            .transpose()?;
        let receipt = self.store.retract(
            &actor,
            id,
            p.reason.unwrap_or(RetractReason::Wrong),
            p.note.as_deref().unwrap_or(""),
            corrected_by,
            p.idempotency_key,
        )?;
        Ok(serde_json::to_value(receipt)?)
    }

    fn invalidate(&self, params: Value) -> Result<Value> {
        let p: InvalidateParams = serde_json::from_value(params)?;
        let actor = self.actor_from(&p.actor);
        let id = self.store.resolve(&p.id)?;
        let at = p.at.as_deref().map(parse_time).transpose()?;
        let receipt = self
            .store
            .invalidate(&actor, id, at, &p.reason, p.idempotency_key)?;
        Ok(serde_json::to_value(receipt)?)
    }

    fn link(&self, params: Value) -> Result<Value> {
        let p: LinkParams = serde_json::from_value(params)?;
        let actor = self.actor_from(&p.actor);
        let from = self.store.resolve(&p.from)?;
        let to = self.store.resolve(&p.to)?;
        let receipt = self.store.link(
            &actor,
            from,
            to,
            p.kind,
            p.note.as_deref(),
            Vec::new(),
            p.idempotency_key,
        )?;
        Ok(serde_json::to_value(receipt)?)
    }

    fn feedback(&self, params: Value) -> Result<Value> {
        let p: FeedbackParams = serde_json::from_value(params)?;
        let actor = self.actor_from(&p.actor);
        let id = self.store.resolve(&p.id)?;
        let receipt = self.store.feedback(
            &actor,
            id,
            p.outcome,
            p.note.as_deref(),
            Vec::new(),
            p.idempotency_key,
        )?;
        Ok(serde_json::to_value(receipt)?)
    }

    fn changes_since(&self, params: Value) -> Result<Value> {
        let p: ChangesParams = serde_json::from_value(params)?;
        let claims = p
            .filter
            .as_ref()
            .and_then(|f| f.claims.as_ref())
            .map(|ids| resolve_many(&self.store, ids))
            .transpose()?;
        let filter = ChangesFilter {
            scopes: p.filter.as_ref().and_then(|f| f.scopes.clone()),
            claims,
            kinds: p.filter.as_ref().and_then(|f| f.kinds.clone()),
        };
        let events = self
            .store
            .changes_since(p.from, p.limit.unwrap_or(100), &filter)?;
        let rows: Vec<Value> = events
            .into_iter()
            .map(|(offset, event)| json!({ "offset": offset, "event": event }))
            .collect();
        Ok(json!({
            "events": rows,
            "next_offset": self.store.next_offset(),
        }))
    }
}

fn ctx_map(pairs: Vec<(String, String)>) -> Context {
    pairs.into_iter().collect()
}

fn resolve_many(store: &Store, ids: &[String]) -> Result<Vec<ClaimId>> {
    ids.iter().map(|id| Ok(store.resolve(id)?)).collect()
}

fn parse_time(text: &str) -> Result<Timestamp> {
    Timestamp::parse_relative(text, Timestamp::now()).map_err(|e| anyhow!("{e}"))
}

#[derive(Debug, Deserialize)]
struct RememberParams {
    statement: String,
    #[serde(default = "default_kind")]
    kind: ClaimKind,
    scope: Scope,
    about: Option<Vec<String>>,
    when: Option<Vec<(String, String)>>,
    fields: Option<Vec<(String, String)>>,
    supersedes: Option<Vec<String>>,
    idempotency_key: Option<String>,
    #[serde(default)]
    actor: ActorParams,
}

fn default_kind() -> ClaimKind {
    ClaimKind::Fact
}

#[derive(Debug, Deserialize)]
struct RecallParams {
    query: String,
    scopes: Option<Vec<Scope>>,
    kinds: Option<Vec<ClaimKind>>,
    about: Option<Vec<String>>,
    ctx: Option<Vec<(String, String)>>,
    history: Option<bool>,
    all_contexts: Option<bool>,
    limit: Option<usize>,
    budget: Option<usize>,
    min_offset: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct ShowParams {
    id: String,
    ctx: Option<Vec<(String, String)>>,
    min_offset: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct RetractParams {
    id: String,
    reason: Option<RetractReason>,
    note: Option<String>,
    corrected_by: Option<String>,
    idempotency_key: Option<String>,
    #[serde(default)]
    actor: ActorParams,
}

#[derive(Debug, Deserialize)]
struct InvalidateParams {
    id: String,
    reason: String,
    at: Option<String>,
    idempotency_key: Option<String>,
    #[serde(default)]
    actor: ActorParams,
}

#[derive(Debug, Deserialize)]
struct LinkParams {
    from: String,
    to: String,
    kind: EdgeKind,
    note: Option<String>,
    idempotency_key: Option<String>,
    #[serde(default)]
    actor: ActorParams,
}

#[derive(Debug, Deserialize)]
struct FeedbackParams {
    id: String,
    outcome: Outcome,
    note: Option<String>,
    idempotency_key: Option<String>,
    #[serde(default)]
    actor: ActorParams,
}

#[derive(Debug, Deserialize)]
struct ChangesParams {
    from: u64,
    limit: Option<usize>,
    filter: Option<ChangesFilterParams>,
}

#[allow(dead_code)]
fn _unused_btreemap(_: BTreeMap<String, String>) {}
