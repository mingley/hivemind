//! In-memory projections of the log: claims, entities, edges, artifacts,
//! idempotency keys, and the lexical index. Each is a pure function of the log,
//! so replaying the log always rebuilds them exactly.
//!
//! The belief policy ([`State::belief`]) derives verification and trust from
//! evidence, edges, and feedback at read time. Beliefs are never stored, so a
//! policy change re-scores history consistently.

use std::collections::{BTreeSet, HashMap};

use hivemind_core::{
    Actor, ActorKind, BELIEF_POLICY, BlobHash, ClaimData, ClaimId, EdgeKind, Event, EventBody,
    EventId, EvidenceRef, Method, Origin, Outcome, RetractReason, Status, Timestamp, TrustTier,
    Verification,
};
use serde::Serialize;

use crate::lexical::LexicalIndex;

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
    pub by: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Invalidation {
    /// When the claim stopped being true in the world.
    pub at: Timestamp,
    pub reason: String,
    pub recorded_at: Timestamp,
    pub by: String,
}

/// One line of support: a principal and the evidence they offered. Lines that
/// share a principal or any evidence collapse into one independent group.
#[derive(Clone, Debug)]
struct Support {
    principal: String,
    evidence: BTreeSet<String>,
}

#[derive(Clone, Debug)]
pub struct ClaimRow {
    pub doc: u32,
    pub id: ClaimId,
    pub data: ClaimData,
    pub author: Actor,
    pub event: EventId,
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
    pub human_verified: bool,
    supports: Vec<Support>,
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
    pub by: String,
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
    pub by: String,
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

#[derive(Debug, Default)]
pub struct State {
    pub(crate) claims: Vec<ClaimRow>,
    by_id: HashMap<ClaimId, u32>,
    pub(crate) by_entity: HashMap<String, Vec<u32>>,
    pub(crate) edges: Vec<EdgeRow>,
    edges_by_claim: HashMap<ClaimId, Vec<u32>>,
    pub(crate) artifacts: HashMap<BlobHash, ArtifactRow>,
    idempotency: HashMap<(String, String), Ack>,
    pub(crate) lexical: LexicalIndex,
    pub(crate) next_offset: u64,
    /// Events that referenced unknown records during replay. Writes are
    /// validated before they reach the log, so this stays zero unless the log
    /// was edited by hand.
    pub(crate) skipped: u64,
}

impl State {
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
        self.idempotency
            .get(&(principal.to_owned(), key.to_owned()))
            .copied()
    }

    /// Edges touching `id`, in either direction.
    pub fn edges_of(&self, id: &ClaimId) -> impl Iterator<Item = &EdgeRow> {
        self.edges_by_claim
            .get(id)
            .into_iter()
            .flatten()
            .map(|i| &self.edges[*i as usize])
    }

    /// Applies one event. Never fails: writes are validated before they reach
    /// the log, and a reference to an unknown record (possible only if the log
    /// was edited by hand) is counted in `skipped` instead of stopping replay.
    pub(crate) fn apply(&mut self, offset: u64, event: &Event) {
        let at = event.recorded_at;
        let by = &event.actor;
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
                        by: by.principal.clone(),
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
                        by: by.principal.clone(),
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
            } => match self.row_mut(claim) {
                Some(row) => match outcome {
                    Outcome::Helpful => {
                        row.feedback.helpful += 1;
                        if !evidence.is_empty() {
                            row.last_validated_at = Some(at);
                            row.supports.push(Support {
                                principal: by.principal.clone(),
                                evidence: evidence.iter().map(EvidenceRef::key).collect(),
                            });
                        }
                        if by.kind == ActorKind::Human {
                            row.human_verified = true;
                            row.last_validated_at = Some(at);
                        }
                    }
                    Outcome::Unhelpful => row.feedback.unhelpful += 1,
                    Outcome::Harmful => row.feedback.harmful += 1,
                    Outcome::NotApplicable => row.feedback.not_applicable += 1,
                },
                None => known = false,
            },
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
                    by: by.principal.clone(),
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
                (by.principal.clone(), key.clone()),
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
        let mut supports = Vec::new();
        if provenance.has_evidence() {
            supports.push(Support {
                principal: event.actor.principal.clone(),
                evidence: provenance.evidence_keys().into_iter().collect(),
            });
        }
        let human = event.actor.kind == ActorKind::Human || provenance.method == Method::Human;
        self.lexical.add(doc, &data.index_text());
        for entity in &data.about {
            self.by_entity.entry(entity.clone()).or_default().push(doc);
        }
        self.by_id.insert(id, doc);
        self.claims.push(ClaimRow {
            doc,
            id,
            data: data.clone(),
            author: event.actor.clone(),
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
            last_validated_at: human.then_some(event.recorded_at),
            human_verified: human,
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
        self.edges.push(EdgeRow {
            from,
            to,
            kind,
            note: note.clone(),
            evidence: evidence.to_vec(),
            by: event.actor.principal.clone(),
            recorded_at: event.recorded_at,
            offset,
        });
        self.edges_by_claim.entry(from).or_default().push(index);
        if from != to {
            self.edges_by_claim.entry(to).or_default().push(index);
        }
        if kind == EdgeKind::Supports {
            // Support from another claim counts as evidence from that claim's author.
            let source = self.row(&from).expect("checked by caller");
            let mut keys: BTreeSet<String> =
                source.data.provenance.evidence_keys().into_iter().collect();
            keys.extend(evidence.iter().map(EvidenceRef::key));
            let principal = source.author.principal.clone();
            if !keys.is_empty() {
                let target = self.row_mut(&to).expect("checked by caller");
                target.supports.push(Support {
                    principal,
                    evidence: keys,
                });
            }
        }
    }

    /// Derives verification and trust under [`BELIEF_POLICY`].
    ///
    /// - **Verified:** confirmed by a human.
    /// - **Disputed:** an active claim contradicts it.
    /// - **Corroborated:** two or more independent evidence groups.
    /// - Trust is capped at `untrusted` for claims derived from external
    ///   content, until another principal corroborates them with independent
    ///   evidence.
    pub fn belief(&self, row: &ClaimRow) -> Belief {
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
        let verification = if row.human_verified {
            Verification::Verified
        } else if !conflicts.is_empty() {
            Verification::Disputed
        } else if independent >= 2 {
            Verification::Corroborated
        } else {
            Verification::Unverified
        };
        let trust = if row.human_verified {
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
        Belief {
            policy: BELIEF_POLICY,
            verification,
            trust,
            independent_support: independent,
            conflicts: conflicts.into_iter().collect(),
        }
    }
}

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
            principal: principal.into(),
            evidence: evidence.iter().map(|e| (*e).to_owned()).collect(),
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
}
