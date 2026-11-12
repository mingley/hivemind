//! Transport-independent handlers over a [`hivemind_engine::Store`].

use std::collections::HashSet;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow};
use hivemind_core::applicability::Context;
use hivemind_core::{
    Actor, ActorKind, ClaimData, ClaimId, ClaimKind, EdgeKind, EvidenceRef, Origin, Outcome,
    RetractReason, Scope, Timestamp, consumption_taints_session, session_taint_key,
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
    /// In-process session taint bits keyed by [`session_taint_key`].
    session_taint: Mutex<HashSet<String>>,
}

impl Service {
    pub fn new(store: Store) -> Self {
        Self {
            store,
            version: env!("CARGO_PKG_VERSION").to_owned(),
            session_taint: Mutex::new(HashSet::new()),
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
        self.dispatch_with_principal(method, params, None)
    }

    /// Shared-service dispatch: when `principal` is set, it overrides any
    /// client-supplied / env TOFU actor principal (token subject wins).
    pub fn dispatch_with_principal(
        &self,
        method: &str,
        params: Value,
        principal: Option<&str>,
    ) -> Result<Value> {
        let params = match principal {
            Some(p) => force_actor_principal(params, p),
            None => params,
        };
        match method {
            "hello" | "ping" => Ok(serde_json::to_value(self.hello())?),
            "remember" => self.remember(params),
            "recall" => self.recall(params),
            "show" => self.show(params),
            "retract" => self.retract(params),
            "forget" => self.forget(params),
            "invalidate" => self.invalidate(params),
            "link" => self.link(params),
            "feedback" => self.feedback(params),
            "promote" => self.promote(params),
            "lease_acquire" => self.lease_acquire(params),
            "lease_renew" => self.lease_renew(params),
            "lease_release" => self.lease_release(params),
            "lease_show" => self.lease_show(params),
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

    fn mark_session_tainted(&self, actor: &Actor) {
        let key = session_taint_key(&actor.principal, actor.session.as_deref());
        if let Ok(mut guard) = self.session_taint.lock() {
            guard.insert(key);
        }
    }

    fn session_is_tainted(&self, actor: &Actor) -> bool {
        let key = session_taint_key(&actor.principal, actor.session.as_deref());
        self.session_taint
            .lock()
            .map(|g| g.contains(&key))
            .unwrap_or(false)
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
        if let Some(origin) = p.origin {
            data.provenance.origin = origin;
        }
        // Session that consumed untrusted/tainted content cannot launder a
        // "self-authored" write into trusted memory: force external origin so
        // the claim stays tainted across replay (same as Origin::External).
        if self.session_is_tainted(&actor) {
            data.provenance.origin = Origin::External;
        }
        if let Some(urls) = p.evidence_urls {
            for url in urls {
                data.provenance.evidence.push(EvidenceRef::Url(url));
            }
        }
        let mut req = hivemind_engine::Remember::new(data);
        if let Some(ids) = p.supersedes {
            req.supersedes = resolve_many(&self.store, &ids)?;
        }
        req.idempotency_key = p.idempotency_key;
        let receipt = self.store.remember(&actor, req)?;
        crate::obs::record_remember();
        Ok(serde_json::to_value(receipt)?)
    }

    fn recall(&self, params: Value) -> Result<Value> {
        let p: RecallParams = serde_json::from_value(params)?;
        let actor = self.actor_from(&p.actor);
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
            entity_boost: true,
            ..Query::default()
        };
        let recall = self.store.recall(&query);
        if recall
            .cards
            .iter()
            .any(|c| consumption_taints_session(c.trust, c.tainted))
        {
            self.mark_session_tainted(&actor);
        }
        crate::obs::record_recall();
        Ok(serde_json::to_value(recall)?)
    }

    fn show(&self, params: Value) -> Result<Value> {
        let p: ShowParams = serde_json::from_value(params)?;
        let actor = self.actor_from(&p.actor);
        self.wait_if_needed(p.min_offset)?;
        let id = self.store.resolve(&p.id)?;
        let ctx = p.ctx.map(ctx_map);
        let detail = self
            .store
            .get(&id, ctx.as_ref())
            .ok_or_else(|| anyhow!("claim {id} missing after resolve"))?;
        if consumption_taints_session(detail.belief.trust, detail.tainted) {
            self.mark_session_tainted(&actor);
        }
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

    fn forget(&self, params: Value) -> Result<Value> {
        let p: ForgetParams = serde_json::from_value(params)?;
        let actor = self.actor_from(&p.actor);
        let id = self.store.resolve(&p.id)?;
        let receipt = self
            .store
            .forget(&actor, id, &p.reason, p.idempotency_key)?;
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

    fn promote(&self, params: Value) -> Result<Value> {
        let p: PromoteParams = serde_json::from_value(params)?;
        let actor = self.actor_from(&p.actor);
        let id = self.store.resolve(&p.id)?;
        let receipt = self.store.promote(&actor, id, p.scope, p.idempotency_key)?;
        Ok(serde_json::to_value(receipt)?)
    }

    fn lease_acquire(&self, params: Value) -> Result<Value> {
        let p: LeaseAcquireParams = serde_json::from_value(params)?;
        let actor = self.actor_from(&p.actor);
        let receipt = self.store.acquire_lease(
            &actor,
            &p.work_item,
            Duration::from_secs(p.ttl_secs.unwrap_or(3600)),
            p.note.as_deref(),
        )?;
        Ok(serde_json::to_value(receipt)?)
    }

    fn lease_renew(&self, params: Value) -> Result<Value> {
        let p: LeaseRenewParams = serde_json::from_value(params)?;
        let actor = self.actor_from(&p.actor);
        let receipt = self.store.renew_lease(
            &actor,
            &p.work_item,
            p.token,
            Duration::from_secs(p.ttl_secs.unwrap_or(3600)),
        )?;
        Ok(serde_json::to_value(receipt)?)
    }

    fn lease_release(&self, params: Value) -> Result<Value> {
        let p: LeaseReleaseParams = serde_json::from_value(params)?;
        let actor = self.actor_from(&p.actor);
        let receipt = self.store.release_lease(&actor, &p.work_item, p.token)?;
        Ok(serde_json::to_value(receipt)?)
    }

    fn lease_show(&self, params: Value) -> Result<Value> {
        let p: LeaseShowParams = serde_json::from_value(params)?;
        if let Some(id) = p.work_item {
            let lease = self
                .store
                .lease(&id)
                .ok_or_else(|| anyhow!("no lease for `{id}`"))?;
            Ok(serde_json::to_value(lease)?)
        } else {
            Ok(serde_json::to_value(self.store.leases())?)
        }
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
    /// Content origin; `external` marks the claim untrusted (scenario 10).
    origin: Option<Origin>,
    /// Optional URL evidence pointers (for example issue comments).
    evidence_urls: Option<Vec<String>>,
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
    /// Used for session-taint keying when this recall returns untrusted content.
    #[serde(default)]
    actor: ActorParams,
}

#[derive(Debug, Deserialize)]
struct ShowParams {
    id: String,
    ctx: Option<Vec<(String, String)>>,
    min_offset: Option<u64>,
    /// Used for session-taint keying when this show returns untrusted content.
    #[serde(default)]
    actor: ActorParams,
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
struct ForgetParams {
    id: String,
    reason: String,
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
struct PromoteParams {
    id: String,
    /// Wider target scope (for example `team:acme` from `project:acme/api`).
    scope: Scope,
    idempotency_key: Option<String>,
    #[serde(default)]
    actor: ActorParams,
}

#[derive(Debug, Deserialize)]
struct LeaseAcquireParams {
    work_item: String,
    ttl_secs: Option<u64>,
    note: Option<String>,
    #[serde(default)]
    actor: ActorParams,
}

#[derive(Debug, Deserialize)]
struct LeaseRenewParams {
    work_item: String,
    token: u64,
    ttl_secs: Option<u64>,
    #[serde(default)]
    actor: ActorParams,
}

#[derive(Debug, Deserialize)]
struct LeaseReleaseParams {
    work_item: String,
    token: u64,
    #[serde(default)]
    actor: ActorParams,
}

#[derive(Debug, Deserialize)]
struct LeaseShowParams {
    work_item: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChangesParams {
    from: u64,
    limit: Option<usize>,
    filter: Option<ChangesFilterParams>,
}

/// Overwrite `params.actor.principal` so shared-path clients cannot spoof identity.
fn force_actor_principal(mut params: Value, principal: &str) -> Value {
    let actor = params
        .as_object_mut()
        .map(|obj| obj.entry("actor").or_insert_with(|| json!({})));
    if let Some(Value::Object(map)) = actor {
        map.insert("principal".to_owned(), Value::String(principal.to_owned()));
    } else if let Some(obj) = params.as_object_mut() {
        obj.insert("actor".to_owned(), json!({ "principal": principal }));
    }
    params
}
