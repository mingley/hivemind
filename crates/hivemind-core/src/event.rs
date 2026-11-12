//! Events: every write to a store is one immutable, versioned event in the log.

use serde::{Deserialize, Serialize};

use crate::claim::{ClaimData, EvidenceRef, Origin};
use crate::ids::{BlobHash, ClaimId, EventId};
use crate::time::Timestamp;

/// Schema version of [`Event`]. Readers upcast older versions; history is never rewritten.
pub const EVENT_VERSION: u16 = 1;

str_enum! {
    pub enum ActorKind {
        Agent => "agent",
        Human => "human",
        Service => "service",
    }
}

/// Who performed an operation. Every write is attributed.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct Actor {
    /// The authenticated identity, such as a user or service account.
    pub principal: String,
    pub kind: ActorKind,
    /// The agent product or name, such as `claude-code`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
}

impl Actor {
    pub fn agent(principal: impl Into<String>) -> Self {
        Self {
            principal: principal.into(),
            kind: ActorKind::Agent,
            agent: None,
            session: None,
        }
    }

    pub fn human(principal: impl Into<String>) -> Self {
        Self {
            kind: ActorKind::Human,
            ..Self::agent(principal)
        }
    }
}

str_enum! {
    pub enum EdgeKind {
        Supports => "supports",
        Contradicts => "contradicts",
        Refines => "refines",
        Related => "related",
        DependsOn => "depends_on",
        /// Near-duplicate claims that differ by applicability context (C5).
        ContrastsWith => "contrasts_with",
    }
}

str_enum! {
    pub enum Outcome {
        /// The claim was applied and worked. With evidence, this is a validated use.
        Helpful => "helpful",
        Unhelpful => "unhelpful",
        /// The claim was wrong or misleading.
        Harmful => "harmful",
        NotApplicable => "not_applicable",
    }
}

str_enum! {
    pub enum RetractReason {
        /// The claim was false.
        Wrong => "wrong",
        /// The evidence never said this; the claim was recorded wrongly.
        Misrecorded => "misrecorded",
        /// Another claim already says this.
        Duplicate => "duplicate",
    }
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Event {
    pub v: u16,
    pub id: EventId,
    /// System (record) time, assigned by the store when the event is committed.
    pub recorded_at: Timestamp,
    pub actor: Actor,
    /// Client-supplied key that makes retries safe: a repeated key returns the
    /// original result instead of writing again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    pub body: EventBody,
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventBody {
    /// A new claim. Listing older claims in `supersedes` replaces them in the
    /// same event, so readers never see both as current.
    ClaimAsserted {
        claim: ClaimId,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        supersedes: Vec<ClaimId>,
        data: Box<ClaimData>,
    },
    /// The claim stopped being true at `at`, with no replacement.
    ClaimInvalidated {
        claim: ClaimId,
        at: Timestamp,
        reason: String,
    },
    /// The claim was wrong or misrecorded.
    ClaimRetracted {
        claim: ClaimId,
        reason: RetractReason,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        note: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        corrected_by: Option<ClaimId>,
    },
    EdgeAdded {
        from: ClaimId,
        to: ClaimId,
        kind: EdgeKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        evidence: Vec<EvidenceRef>,
    },
    FeedbackRecorded {
        claim: ClaimId,
        outcome: Outcome,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        evidence: Vec<EvidenceRef>,
    },
    /// Recovery removed a torn, unacknowledged write from the end of the log
    /// when the store was opened. Recorded so the repair is never silent.
    Recovered { truncated_bytes: u64 },
    /// Records an artifact written to the blob store. The blob is durable before
    /// this event is appended.
    ArtifactStored {
        hash: BlobHash,
        size: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        media_type: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        origin: Origin,
    },
    /// Exclusive lease on a work item, with a fencing token.
    LeaseAcquired {
        work_item: String,
        token: u64,
        expires_at: Timestamp,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
    },
    /// Extends an held lease; token must match the current fence.
    LeaseRenewed {
        work_item: String,
        token: u64,
        expires_at: Timestamp,
    },
    /// Releases a lease; token must match the current fence.
    LeaseReleased { work_item: String, token: u64 },
    /// Privacy erasure: claim payload is (or will be) purged from the log,
    /// projections, and exclusive blobs. Only this tombstone plus a redacted
    /// `ClaimAsserted` stub remain durable.
    ClaimForgotten {
        claim: ClaimId,
        reason: String,
        /// Blob hashes deleted with this claim because nothing else referenced them.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        purged_blobs: Vec<BlobHash>,
    },
}

impl EventBody {
    pub fn type_name(&self) -> &'static str {
        match self {
            EventBody::ClaimAsserted { .. } => "claim_asserted",
            EventBody::ClaimInvalidated { .. } => "claim_invalidated",
            EventBody::ClaimRetracted { .. } => "claim_retracted",
            EventBody::EdgeAdded { .. } => "edge_added",
            EventBody::FeedbackRecorded { .. } => "feedback_recorded",
            EventBody::ArtifactStored { .. } => "artifact_stored",
            EventBody::Recovered { .. } => "recovered",
            EventBody::LeaseAcquired { .. } => "lease_acquired",
            EventBody::LeaseRenewed { .. } => "lease_renewed",
            EventBody::LeaseReleased { .. } => "lease_released",
            EventBody::ClaimForgotten { .. } => "claim_forgotten",
        }
    }

    /// The claim an event is about, if any.
    pub fn claim(&self) -> Option<ClaimId> {
        match self {
            EventBody::ClaimAsserted { claim, .. }
            | EventBody::ClaimInvalidated { claim, .. }
            | EventBody::ClaimRetracted { claim, .. }
            | EventBody::FeedbackRecorded { claim, .. }
            | EventBody::ClaimForgotten { claim, .. } => Some(*claim),
            EventBody::EdgeAdded { from, .. } => Some(*from),
            EventBody::ArtifactStored { .. }
            | EventBody::Recovered { .. }
            | EventBody::LeaseAcquired { .. }
            | EventBody::LeaseRenewed { .. }
            | EventBody::LeaseReleased { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claim::ClaimKind;

    #[test]
    fn events_round_trip_through_json() {
        let event = Event {
            v: EVENT_VERSION,
            id: EventId::generate(),
            recorded_at: Timestamp::from_millis(1_700_000_000_000),
            actor: Actor::agent("mingley"),
            idempotency_key: Some("k1".into()),
            body: EventBody::ClaimAsserted {
                claim: ClaimId::generate(),
                supersedes: vec![ClaimId::generate()],
                data: Box::new(ClaimData::new(
                    ClaimKind::Decision,
                    "Use our own log",
                    "project:hivemind".parse().unwrap(),
                )),
            },
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""type":"claim_asserted""#));
        let back: Event = serde_json::from_str(&json).unwrap();
        assert_eq!(back, event);
    }
}
