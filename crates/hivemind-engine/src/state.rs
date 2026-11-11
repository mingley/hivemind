//! In-memory projections of the log: claims, entities, edges, artifacts,
//! idempotency keys, and the lexical index. Each is a pure function of the log,
//! so replaying the log always rebuilds them exactly.
//!
//! Claim projections are a compact **keydir**: filter and rank fields stay in
//! RAM, with a short statement cache for cards. Full [`ClaimData`] is loaded
//! from the log by stored offset on demand (show / detail).
//!
//! Scopes, principals, and entity keys are interned as [`Arc<str>`] so repeated
//! values share one allocation.
//!
//! The belief policy ([`State::belief`]) derives verification and trust from
//! evidence, edges, and feedback at read time. Beliefs are never stored, so a
//! policy change re-scores history consistently.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use hivemind_core::{
    Actor, ActorKind, BELIEF_POLICY, BlobHash, ClaimData, ClaimId, ClaimKind, EdgeKind, Event,
    EventBody, EventId, EvidenceRef, Origin, Outcome, RetractReason, Scope, Status, Timestamp,
    TrustTier, Verification,
};
use serde::Serialize;

use crate::lexical::{DEFAULT_MAX_TERMS, LexicalIndex};

/// Characters kept in the in-memory statement cache for cards and edge previews.
pub const STATEMENT_CACHE_CHARS: usize = 280;

#[derive(Clone, Debug, Default, Serialize)]
pub struct FeedbackTally {
    pub helpful: u32,
    pub unhelpful: u32,
    pub harmful: u32,
    pub not_applicable: u32,
}

#[derive(Clone, Debug, Serialize)]
pub struct Retraction {
    pub reason: RetractReason,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub corrected_by: Option<ClaimId>,
    pub recorded_at: Timestamp,
    pub by: Arc<str>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Invalidation {
    /// When the claim stopped being true in the world.
    pub at: Timestamp,
    pub reason: String,
    pub recorded_at: Timestamp,
    pub by: Arc<str>,
}

/// One line of support: a principal and the evidence they offered. Lines that
/// share a principal or any evidence collapse into one independent group.
#[derive(Clone, Debug)]
struct Support {
    principal: Arc<str>,
    evidence: BTreeSet<Arc<str>>,
}

/// Compact claim projection (keydir). Full claim bodies live in the log at
/// [`ClaimRow::offset`] and are loaded on demand for show/detail.
#[derive(Clone, Debug)]
pub struct ClaimRow {
    pub doc: u32,
    pub id: ClaimId,
    pub kind: ClaimKind,
    pub scope: Scope,
    pub about: Vec<Arc<str>>,
    pub applies_when: BTreeMap<String, String>,
    pub review_by: Option<Timestamp>,
    /// Short statement for recall cards; may be truncated.
    pub statement_cache: String,
    pub statement_truncated: bool,
    pub evidence_count: usize,
    pub citations_count: usize,
    pub derived_from: Vec<ClaimId>,
    /// Provenance evidence keys, used when crediting support edges.
    evidence_keys: BTreeSet<Arc<str>>,
    pub author_principal: Arc<str>,
    pub author_kind: ActorKind,
    pub author_agent: Option<Arc<str>>,
    pub author_session: Option<Arc<str>>,
    pub event: EventId,
    /// Log offset of the `ClaimAsserted` event; used to reload full [`ClaimData`].
    pub offset: u64,
    pub recorded_at: Timestamp,
    pub status: Status,
    pub supersedes: Vec<ClaimId>,
    pub superseded_by: Option<ClaimId>,
    pub superseded_at: Option<Timestamp>,
    pub retraction: Option<Retraction>,
    pub invalidation: Option<Invalidation>,
    /// Derived from external content, directly or through `derived_from`.
    pub tainted: bool,
    pub feedback: FeedbackTally,
    /// Last validated use: helpful feedback with evidence, or a human confirmation.
    pub last_validated_at: Option<Timestamp>,
    /// The author was a human. Never inferred from writer-chosen labels.
    pub authored_by_human: bool,
    /// A human confirmed the claim through helpful feedback.
    pub reviewed_by_human: bool,
    supports: Vec<Support>,
}

impl ClaimRow {
    pub fn author(&self) -> Actor {
        Actor {
            principal: self.author_principal.to_string(),
            kind: self.author_kind,
            agent: self.author_agent.as_ref().map(|a| a.to_string()),
            session: self.author_session.as_ref().map(|a| a.to_string()),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct EdgeRow {
    pub from: ClaimId,
    pub to: ClaimId,
    pub kind: EdgeKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<EvidenceRef>,
    pub by: Arc<str>,
    pub recorded_at: Timestamp,
    pub offset: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct ArtifactRow {
    pub hash: BlobHash,
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub origin: Origin,
    pub recorded_at: Timestamp,
    pub by: Arc<str>,
}

/// The original result of an idempotent write.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Ack {
    pub event: EventId,
    pub offset: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claim: Option<ClaimId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Belief {
    pub policy: &'static str,
    pub verification: Verification,
    pub trust: TrustTier,
    /// Independent evidence groups supporting the claim.
    pub independent_support: usize,
    /// Active claims that contradict this one.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub conflicts: Vec<ClaimId>,
}

/// Shared string table for scopes, principals, and entity keys.
#[derive(Debug, Default)]
pub struct Interner {
    table: HashMap<Arc<str>, ()>,
}

impl Interner {
    pub fn intern(&mut self, s: &str) -> Arc<str> {
        if let Some((existing, _)) = self.table.get_key_value(s) {
            return Arc::clone(existing);
        }
        let arc: Arc<str> = Arc::from(s);
        self.table.insert(Arc::clone(&arc), ());
        arc
    }

    pub fn len(&self) -> usize {
        self.table.len()
    }

    pub fn is_empty(&self) -> bool {
        self.table.is_empty()
    }
}

#[derive(Debug)]
pub struct State {
    pub(crate) claims: Vec<ClaimRow>,
    by_id: HashMap<ClaimId, u32>,
    pub(crate) by_entity: HashMap<Arc<str>, Vec<u32>>,
    pub(crate) edges: Vec<EdgeRow>,
    edges_by_claim: HashMap<ClaimId, Vec<u32>>,
    pub(crate) artifacts: HashMap<BlobHash, ArtifactRow>,
    idempotency: HashMap<(Arc<str>, String), Ack>,
    pub(crate) lexical: LexicalIndex,
    pub(crate) intern: Interner,
    pub(crate) next_offset: u64,
    /// `(offset, truncated bytes)` of every recorded recovery.
    pub(crate) recoveries: Vec<(u64, u64)>,
    /// Events that referenced unknown records during replay. Writes are
    /// validated before they reach the log, so this stays zero unless the log
    /// was edited by hand.
    pub(crate) skipped: u64,
}

impl Default for State {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_TERMS)
    }
}

impl State {
    pub fn new(max_terms: usize) -> Self {
        Self {
            claims: Vec::new(),
            by_id: HashMap::new(),
            by_entity: HashMap::new(),
            edges: Vec::new(),
            edges_by_claim: HashMap::new(),
            artifacts: HashMap::new(),
            idempotency: HashMap::new(),
            lexical: LexicalIndex::with_max_terms(max_terms),
            intern: Interner::default(),
            next_offset: 0,
            recoveries: Vec::new(),
            skipped: 0,
        }
    }

    pub fn row(&self, id: &ClaimId) -> Option<&ClaimRow> {
        self.by_id.get(id).map(|doc| &self.claims[*doc as usize])
    }

    fn row_mut(&mut self, id: &ClaimId) -> Option<&mut ClaimRow> {
        self.by_id
            .get(id)
            .copied()
            .map(|doc| &mut self.claims[doc as usize])
    }

    pub fn claims(&self) -> &[ClaimRow] {
        &self.claims
    }

    pub fn ack(&self, principal: &str, key: &str) -> Option<Ack> {
        let p = self.lookup_interned(principal)?;
        self.idempotency.get(&(p, key.to_owned())).copied()
    }

    fn lookup_interned(&self, s: &str) -> Option<Arc<str>> {
        self.intern
            .table
            .get_key_value(s)
            .map(|(k, _)| Arc::clone(k))
    }

    /// Edges touching `id`, in either direction.
    pub fn edges_of(&self, id: &ClaimId) -> impl Iterator<Item = &EdgeRow> {
        self.edges_by_claim
            .get(id)
            .into_iter()
            .flatten()
            .map(|i| &self.edges[*i as usize])
    }

    fn intern_scope(&mut self, scope: &Scope) -> Scope {
        match scope.name() {
            None => Scope::Global,
            Some(name) => scope.clone().with_name(self.intern.intern(name)),
        }
    }

    /// Applies one event. Never fails: writes are validated before they reach
    /// the log, and a reference to an unknown record (possible only if the log
    /// was edited by hand) is counted in `skipped` instead of stopping replay.
    pub(crate) fn apply(&mut self, offset: u64, event: &Event) {
        let at = event.recorded_at;
        let by = self.intern.intern(&event.actor.principal);
        let mut known = true;
        match &event.body {
            EventBody::ClaimAsserted {
                claim,
                supersedes,
                data,
            } => {
                if self.by_id.contains_key(claim) {
                    known = false;
                } else {
                    self.insert_claim(offset, event, *claim, supersedes, data);
                    for old in supersedes {
                        if let Some(row) = self.row_mut(old)
                            && row.status == Status::Active
                        {
                            row.status = Status::Superseded;
                            row.superseded_by = Some(*claim);
                            row.superseded_at = Some(at);
                        }
                    }
                }
            }
            EventBody::ClaimInvalidated {
                claim,
                at: when,
                reason,
            } => match self.row_mut(claim) {
                Some(row) => {
                    row.status = Status::Invalidated;
                    row.invalidation = Some(Invalidation {
                        at: *when,
                        reason: reason.clone(),
                        recorded_at: at,
                        by: Arc::clone(&by),
                    });
                }
                None => known = false,
            },
            EventBody::ClaimRetracted {
                claim,
                reason,
                note,
                corrected_by,
            } => match self.row_mut(claim) {
                Some(row) => {
                    row.status = Status::Retracted;
                    row.retraction = Some(Retraction {
                        reason: *reason,
                        note: note.clone(),
                        corrected_by: *corrected_by,
                        recorded_at: at,
                        by: Arc::clone(&by),
                    });
                }
                None => known = false,
            },
            EventBody::EdgeAdded {
                from,
                to,
                kind,
                note,
                evidence,
            } => {
                if self.row(from).is_some() && self.row(to).is_some() {
                    self.insert_edge(offset, event, *from, *to, *kind, note, evidence);
                } else {
                    known = false;
                }
            }
            EventBody::FeedbackRecorded {
                claim,
                outcome,
                evidence,
                ..
            } => {
                if self.row(claim).is_none() {
                    known = false;
                } else {
                    match outcome {
                        Outcome::Helpful => {
                            let keys: BTreeSet<Arc<str>> = evidence
                                .iter()
                                .map(|e| self.intern.intern(&e.key()))
                                .collect();
                            let principal = Arc::clone(&by);
                            let is_human = event.actor.kind == ActorKind::Human;
                            let row = self.row_mut(claim).expect("checked above");
                            row.feedback.helpful += 1;
                            if !keys.is_empty() {
                                row.last_validated_at = Some(at);
                                row.supports.push(Support {
                                    principal,
                                    evidence: keys,
                                });
                            }
                            if is_human {
                                row.reviewed_by_human = true;
                                row.last_validated_at = Some(at);
                            }
                        }
                        Outcome::Unhelpful => {
                            self.row_mut(claim)
                                .expect("checked above")
                                .feedback
                                .unhelpful += 1;
                        }
                        Outcome::Harmful => {
                            self.row_mut(claim).expect("checked above").feedback.harmful += 1;
                        }
                        Outcome::NotApplicable => {
                            self.row_mut(claim)
                                .expect("checked above")
                                .feedback
                                .not_applicable += 1;
                        }
                    }
                }
            }
            EventBody::Recovered { truncated_bytes } => {
                self.recoveries.push((offset, *truncated_bytes));
            }
            EventBody::ArtifactStored {
                hash,
                size,
                media_type,
                name,
                origin,
            } => {
                self.artifacts.entry(*hash).or_insert_with(|| ArtifactRow {
                    hash: *hash,
                    size: *size,
                    media_type: media_type.clone(),
                    name: name.clone(),
                    origin: *origin,
                    recorded_at: at,
                    by: Arc::clone(&by),
                });
            }
        }
        if !known {
            self.skipped += 1;
        }
        if let Some(key) = &event.idempotency_key {
            let claim = match &event.body {
                EventBody::ClaimAsserted { claim, .. } => Some(*claim),
                other => other.claim(),
            };
            self.idempotency.insert(
                (by, key.clone()),
                Ack {
                    event: event.id,
                    offset,
                    claim,
                },
            );
        }
        self.next_offset = offset + 1;
    }

    fn insert_claim(
        &mut self,
        offset: u64,
        event: &Event,
        id: ClaimId,
        supersedes: &[ClaimId],
        data: &ClaimData,
    ) {
        let doc = self.claims.len() as u32;
        let provenance = &data.provenance;
        let tainted = provenance.origin == Origin::External
            || provenance
                .derived_from
                .iter()
                .any(|input| self.row(input).is_some_and(|r| r.tainted));
        let principal = self.intern.intern(&event.actor.principal);
        let evidence_keys: BTreeSet<Arc<str>> = provenance
            .evidence_keys()
            .into_iter()
            .map(|k| self.intern.intern(&k))
            .collect();
        let mut supports = Vec::new();
        if provenance.has_evidence() {
            supports.push(Support {
                principal: Arc::clone(&principal),
                evidence: evidence_keys.clone(),
            });
        }
        // `provenance.method` is a label the writer chooses, so it never grants trust.
        let authored_by_human = event.actor.kind == ActorKind::Human;
        self.lexical.add(doc, &data.index_text());
        let about: Vec<Arc<str>> = data
            .about
            .iter()
            .map(|entity| self.intern.intern(entity))
            .collect();
        for entity in &about {
            self.by_entity
                .entry(Arc::clone(entity))
                .or_default()
                .push(doc);
        }
        let scope = self.intern_scope(&data.scope);
        let (statement_cache, statement_truncated) =
            truncate_statement(&data.statement, STATEMENT_CACHE_CHARS);
        let author_agent = event.actor.agent.as_deref().map(|a| self.intern.intern(a));
        let author_session = event
            .actor
            .session
            .as_deref()
            .map(|s| self.intern.intern(s));
        self.by_id.insert(id, doc);
        self.claims.push(ClaimRow {
            doc,
            id,
            kind: data.kind,
            scope,
            about,
            applies_when: data.applies_when.clone(),
            review_by: data.review_by,
            statement_cache,
            statement_truncated,
            evidence_count: provenance.evidence.len(),
            citations_count: provenance.citations.len(),
            derived_from: provenance.derived_from.clone(),
            evidence_keys,
            author_principal: principal,
            author_kind: event.actor.kind,
            author_agent,
            author_session,
            event: event.id,
            offset,
            recorded_at: event.recorded_at,
            status: Status::Active,
            supersedes: supersedes.to_vec(),
            superseded_by: None,
            superseded_at: None,
            retraction: None,
            invalidation: None,
            tainted,
            feedback: FeedbackTally::default(),
            last_validated_at: (authored_by_human && !tainted).then_some(event.recorded_at),
            authored_by_human,
            reviewed_by_human: false,
            supports,
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_edge(
        &mut self,
        offset: u64,
        event: &Event,
        from: ClaimId,
        to: ClaimId,
        kind: EdgeKind,
        note: &Option<String>,
        evidence: &[EvidenceRef],
    ) {
        let index = self.edges.len() as u32;
        let by = self.intern.intern(&event.actor.principal);
        self.edges.push(EdgeRow {
            from,
            to,
            kind,
            note: note.clone(),
            evidence: evidence.to_vec(),
            by: Arc::clone(&by),
            recorded_at: event.recorded_at,
            offset,
        });
        self.edges_by_claim.entry(from).or_default().push(index);
        if from != to {
            self.edges_by_claim.entry(to).or_default().push(index);
        }
        if kind == EdgeKind::Supports {
            // Support is credited to whoever adds the edge. The source claim's
            // evidence counts only when that same principal wrote it, so nobody
            // can borrow someone else's evidence to corroborate their own claim.
            let (same_author, source_keys) = {
                let source = self.row(&from).expect("checked by caller");
                (
                    source.author_principal.as_ref() == by.as_ref(),
                    source.evidence_keys.clone(),
                )
            };
            let mut keys: BTreeSet<Arc<str>> = evidence
                .iter()
                .map(|e| self.intern.intern(&e.key()))
                .collect();
            if same_author {
                keys.extend(source_keys);
            }
            if !keys.is_empty() {
                let target = self.row_mut(&to).expect("checked by caller");
                target.supports.push(Support {
                    principal: by,
                    evidence: keys,
                });
            }
        }
    }

    /// Derives verification and trust under [`BELIEF_POLICY`] (`belief-v1`):
    ///
    /// - **Human review:** a human authored the claim (and it isn't derived
    ///   from external content), or a human confirmed it through helpful
    ///   feedback. That makes it `verified` with `human` trust.
    /// - **Disputed:** otherwise, an active claim contradicts it.
    /// - **Corroborated:** two or more independent evidence groups.
    /// - **Taint:** claims derived from external content are `untrusted` until
    ///   another principal corroborates them, or a human reviews them.
    /// - **Derived minimum:** without human review, a derived claim is never
    ///   trusted more than its least trusted input.
    pub fn belief(&self, row: &ClaimRow) -> Belief {
        self.belief_at(row, 0)
    }

    fn belief_at(&self, row: &ClaimRow, depth: usize) -> Belief {
        let conflicts: BTreeSet<ClaimId> = self
            .edges_of(&row.id)
            .filter(|edge| edge.kind == EdgeKind::Contradicts)
            .map(|edge| {
                if edge.from == row.id {
                    edge.to
                } else {
                    edge.from
                }
            })
            .filter(|other| {
                *other != row.id && self.row(other).is_some_and(|o| o.status == Status::Active)
            })
            .collect();
        let independent = independent_groups(&row.supports);
        let human_review = row.reviewed_by_human || (row.authored_by_human && !row.tainted);
        let verification = if human_review {
            Verification::Verified
        } else if !conflicts.is_empty() {
            Verification::Disputed
        } else if independent >= 2 {
            Verification::Corroborated
        } else {
            Verification::Unverified
        };
        let mut trust = if human_review {
            TrustTier::Human
        } else if row.tainted {
            if independent >= 2 {
                TrustTier::Agent
            } else {
                TrustTier::Untrusted
            }
        } else if independent >= 2 {
            TrustTier::Corroborated
        } else {
            TrustTier::Agent
        };
        if !human_review {
            for input in &row.derived_from {
                let input_trust = match self.row(input) {
                    Some(r) if depth < MAX_DERIVATION_DEPTH => self.belief_at(r, depth + 1).trust,
                    _ => TrustTier::Untrusted,
                };
                trust = trust.min(input_trust);
            }
        }
        Belief {
            policy: BELIEF_POLICY,
            verification,
            trust,
            independent_support: independent,
            conflicts: conflicts.into_iter().collect(),
        }
    }
}

fn truncate_statement(text: &str, max_chars: usize) -> (String, bool) {
    if text.chars().count() <= max_chars {
        return (text.to_owned(), false);
    }
    let mut cut: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    cut.push('…');
    (cut, true)
}

/// Derivation chains deeper than this are treated as untrusted rather than
/// walked further.
const MAX_DERIVATION_DEPTH: usize = 8;

/// Counts groups of supports that share neither a principal nor any evidence.
/// Ten agents restating one log line are one group.
fn independent_groups(supports: &[Support]) -> usize {
    let mut parent: Vec<usize> = (0..supports.len()).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for i in 0..supports.len() {
        for j in i + 1..supports.len() {
            let related = supports[i].principal == supports[j].principal
                || !supports[i].evidence.is_disjoint(&supports[j].evidence);
            if related {
                let (a, b) = (root(&mut parent, i), root(&mut parent, j));
                parent[a] = b;
            }
        }
    }
    (0..supports.len())
        .filter(|i| root(&mut parent, *i) == *i)
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn support(principal: &str, evidence: &[&str]) -> Support {
        Support {
            principal: Arc::from(principal),
            evidence: evidence.iter().map(|e| Arc::from(*e)).collect(),
        }
    }

    #[test]
    fn repetition_is_not_corroboration() {
        assert_eq!(independent_groups(&[]), 0);
        assert_eq!(
            independent_groups(&[support("a", &["log1"]), support("a", &["log2"])]),
            1
        );
        assert_eq!(
            independent_groups(&[support("a", &["log1"]), support("b", &["log1"])]),
            1,
            "shared evidence is one group"
        );
        assert_eq!(
            independent_groups(&[
                support("a", &["log1"]),
                support("b", &["log2"]),
                support("c", &["log2", "log3"]),
            ]),
            2
        );
    }

    #[test]
    fn interner_deduplicates_equal_strings() {
        let mut intern = Interner::default();
        let a = intern.intern("project:acme/api");
        let b = intern.intern("project:acme/api");
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!(intern.len(), 1);
        let _ = intern.intern("other");
        assert_eq!(intern.len(), 2);
    }

    #[test]
    fn statement_cache_truncates_long_text() {
        let long = "x".repeat(STATEMENT_CACHE_CHARS + 10);
        let (cache, truncated) = truncate_statement(&long, STATEMENT_CACHE_CHARS);
        assert!(truncated);
        assert_eq!(cache.chars().count(), STATEMENT_CACHE_CHARS);
        assert!(cache.ends_with('…'));
    }
}
