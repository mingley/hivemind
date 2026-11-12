//! Event encoding. The first byte names the codec, so the format can evolve
//! without rewriting history; readers keep every codec they ever wrote.
//!
//! - [`JSON_V1`] (1): serde_json of [`Event`]. Default for new appends. Golden
//!   fixtures under `tests/fixtures/codec/` freeze its on-disk bytes.
//! - [`POSTCARD_V1`] (2): compact postcard of a positional wire shape that
//!   round-trips the same [`Event`] values. Postcard cannot host the JSON
//!   internally-tagged `EventBody` or `skip_serializing_if` layouts, so the
//!   wire types always serialize every field and keep free-form claim payloads
//!   as JSON bytes inside the postcard frame.
//!
//! Codecs may be mixed in one log: each event carries its own tag, and
//! [`decode`] accepts every known tag. New writes use
//! [`Options::event_codec`](crate::store::Options::event_codec) /
//! `HIVEMIND_EVENT_CODEC` (see `docs/operations.md`).

use hivemind_core::Event;

use crate::error::{Error, Result};

/// serde_json encoding of [`Event`]. Readable with standard tools, and struct
/// fields serialize in declaration order, so encodings are deterministic.
pub const JSON_V1: u8 = 1;

/// Compact postcard encoding of the same logical [`Event`] (positional wire).
pub const POSTCARD_V1: u8 = 2;

/// Codec used when appending new events.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum EventCodec {
    #[default]
    JsonV1 = JSON_V1,
    PostcardV1 = POSTCARD_V1,
}

impl EventCodec {
    pub const fn tag(self) -> u8 {
        self as u8
    }

    pub fn from_tag(tag: u8) -> Option<Self> {
        match tag {
            JSON_V1 => Some(Self::JsonV1),
            POSTCARD_V1 => Some(Self::PostcardV1),
            _ => None,
        }
    }

    /// Parse operator names: `json` / `json_v1` / `1`, `postcard` / `postcard_v1` / `2`.
    pub fn parse(name: &str) -> Result<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "json" | "json_v1" | "1" => Ok(Self::JsonV1),
            "postcard" | "postcard_v1" | "2" => Ok(Self::PostcardV1),
            other => Err(Error::Invalid(format!(
                "unknown event codec `{other}`; expected json, postcard, 1, or 2"
            ))),
        }
    }

    /// `HIVEMIND_EVENT_CODEC` when set and non-empty; otherwise [`EventCodec::JsonV1`].
    pub fn from_env() -> Result<Self> {
        match std::env::var("HIVEMIND_EVENT_CODEC") {
            Ok(value) if !value.trim().is_empty() => Self::parse(&value),
            _ => Ok(Self::JsonV1),
        }
    }
}

/// Encode with the default JSON v1 codec.
pub fn encode(event: &Event) -> Result<Vec<u8>> {
    encode_with(EventCodec::JsonV1, event)
}

pub fn encode_with(codec: EventCodec, event: &Event) -> Result<Vec<u8>> {
    match codec {
        EventCodec::JsonV1 => {
            let mut bytes = vec![JSON_V1];
            serde_json::to_writer(&mut bytes, event).map_err(|e| Error::Invalid(e.to_string()))?;
            Ok(bytes)
        }
        EventCodec::PostcardV1 => {
            let wire = postcard_wire::WireEvent::from(event);
            let mut bytes = vec![POSTCARD_V1];
            let body = postcard::to_stdvec(&wire).map_err(|e| Error::Invalid(e.to_string()))?;
            bytes.extend_from_slice(&body);
            Ok(bytes)
        }
    }
}

pub fn decode(offset: u64, bytes: &[u8]) -> Result<Event> {
    match bytes.split_first() {
        Some((&JSON_V1, json)) => serde_json::from_slice(json).map_err(|e| Error::Decode {
            offset,
            reason: e.to_string(),
        }),
        Some((&POSTCARD_V1, body)) => {
            let wire: postcard_wire::WireEvent =
                postcard::from_bytes(body).map_err(|e| Error::Decode {
                    offset,
                    reason: e.to_string(),
                })?;
            Ok(wire
                .into_event()
                .map_err(|e| Error::Decode { offset, reason: e })?)
        }
        Some((codec, _)) => Err(Error::Decode {
            offset,
            reason: format!("unknown codec {codec}"),
        }),
        None => Err(Error::Decode {
            offset,
            reason: "empty event".into(),
        }),
    }
}

/// Tag byte of an encoded event, if any.
pub fn peek_codec(bytes: &[u8]) -> Option<u8> {
    bytes.first().copied()
}

/// Positional postcard wire for [`Event`]. Kept private so JSON remains the
/// human-readable on-disk shape; only the tag + these bytes are frozen.
mod postcard_wire {
    use std::collections::BTreeMap;

    use hivemind_core::{
        Actor, ActorKind, BlobHash, Citation, ClaimData, ClaimId, ClaimKind, EdgeKind, Event,
        EventBody, EventId, EvidenceRef, Method, Origin, Outcome, Provenance, RetractReason, Scope,
        Timestamp,
    };
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize)]
    pub(super) struct WireEvent {
        v: u16,
        id: EventId,
        recorded_at: Timestamp,
        actor: WireActor,
        idempotency_key: Option<String>,
        body: WireEventBody,
    }

    #[derive(Serialize, Deserialize)]
    struct WireActor {
        principal: String,
        kind: ActorKind,
        agent: Option<String>,
        session: Option<String>,
    }

    #[derive(Serialize, Deserialize)]
    enum WireEventBody {
        ClaimAsserted {
            claim: ClaimId,
            supersedes: Vec<ClaimId>,
            /// Boxed to keep `WireEventBody` small (`large_enum_variant`).
            data: Box<WireClaimData>,
        },
        ClaimInvalidated {
            claim: ClaimId,
            at: Timestamp,
            reason: String,
        },
        ClaimRetracted {
            claim: ClaimId,
            reason: RetractReason,
            note: String,
            corrected_by: Option<ClaimId>,
        },
        EdgeAdded {
            from: ClaimId,
            to: ClaimId,
            kind: EdgeKind,
            note: Option<String>,
            evidence: Vec<EvidenceRef>,
        },
        FeedbackRecorded {
            claim: ClaimId,
            outcome: Outcome,
            note: Option<String>,
            evidence: Vec<EvidenceRef>,
        },
        Recovered {
            truncated_bytes: u64,
        },
        ArtifactStored {
            hash: BlobHash,
            size: u64,
            media_type: Option<String>,
            name: Option<String>,
            origin: Origin,
        },
        LeaseAcquired {
            work_item: String,
            token: u64,
            expires_at: Timestamp,
            note: Option<String>,
        },
        LeaseRenewed {
            work_item: String,
            token: u64,
            expires_at: Timestamp,
        },
        LeaseReleased {
            work_item: String,
            token: u64,
        },
        ClaimForgotten {
            claim: ClaimId,
            reason: String,
            purged_blobs: Vec<BlobHash>,
        },
    }

    #[derive(Serialize, Deserialize)]
    struct WireClaimData {
        kind: ClaimKind,
        statement: String,
        scope: Scope,
        about: Vec<String>,
        /// `serde_json::Map` is not postcard-safe; keep free-form payload as JSON bytes.
        payload_json: Vec<u8>,
        applies_when: Vec<(String, String)>,
        valid_from: Option<Timestamp>,
        valid_to: Option<Timestamp>,
        review_by: Option<Timestamp>,
        provenance: WireProvenance,
    }

    #[derive(Serialize, Deserialize)]
    struct WireProvenance {
        method: Method,
        origin: Origin,
        evidence: Vec<EvidenceRef>,
        citations: Vec<WireCitation>,
        derived_from: Vec<ClaimId>,
    }

    #[derive(Serialize, Deserialize)]
    struct WireCitation {
        path: String,
        repo: Option<String>,
        commit: Option<String>,
        start_line: Option<u32>,
        end_line: Option<u32>,
        content_hash: Option<String>,
    }

    impl From<&Event> for WireEvent {
        fn from(event: &Event) -> Self {
            Self {
                v: event.v,
                id: event.id,
                recorded_at: event.recorded_at,
                actor: WireActor::from(&event.actor),
                idempotency_key: event.idempotency_key.clone(),
                body: WireEventBody::from(&event.body),
            }
        }
    }

    impl WireEvent {
        pub(super) fn into_event(self) -> Result<Event, String> {
            Ok(Event {
                v: self.v,
                id: self.id,
                recorded_at: self.recorded_at,
                actor: self.actor.into(),
                idempotency_key: self.idempotency_key,
                body: self.body.into_body()?,
            })
        }
    }

    impl From<&Actor> for WireActor {
        fn from(actor: &Actor) -> Self {
            Self {
                principal: actor.principal.clone(),
                kind: actor.kind,
                agent: actor.agent.clone(),
                session: actor.session.clone(),
            }
        }
    }

    impl From<WireActor> for Actor {
        fn from(actor: WireActor) -> Self {
            Self {
                principal: actor.principal,
                kind: actor.kind,
                agent: actor.agent,
                session: actor.session,
            }
        }
    }

    impl From<&EventBody> for WireEventBody {
        fn from(body: &EventBody) -> Self {
            match body {
                EventBody::ClaimAsserted {
                    claim,
                    supersedes,
                    data,
                } => Self::ClaimAsserted {
                    claim: *claim,
                    supersedes: supersedes.clone(),
                    data: Box::new(WireClaimData::from(data.as_ref())),
                },
                EventBody::ClaimInvalidated { claim, at, reason } => Self::ClaimInvalidated {
                    claim: *claim,
                    at: *at,
                    reason: reason.clone(),
                },
                EventBody::ClaimRetracted {
                    claim,
                    reason,
                    note,
                    corrected_by,
                } => Self::ClaimRetracted {
                    claim: *claim,
                    reason: *reason,
                    note: note.clone(),
                    corrected_by: *corrected_by,
                },
                EventBody::EdgeAdded {
                    from,
                    to,
                    kind,
                    note,
                    evidence,
                } => Self::EdgeAdded {
                    from: *from,
                    to: *to,
                    kind: *kind,
                    note: note.clone(),
                    evidence: evidence.clone(),
                },
                EventBody::FeedbackRecorded {
                    claim,
                    outcome,
                    note,
                    evidence,
                } => Self::FeedbackRecorded {
                    claim: *claim,
                    outcome: *outcome,
                    note: note.clone(),
                    evidence: evidence.clone(),
                },
                EventBody::Recovered { truncated_bytes } => Self::Recovered {
                    truncated_bytes: *truncated_bytes,
                },
                EventBody::ArtifactStored {
                    hash,
                    size,
                    media_type,
                    name,
                    origin,
                } => Self::ArtifactStored {
                    hash: *hash,
                    size: *size,
                    media_type: media_type.clone(),
                    name: name.clone(),
                    origin: *origin,
                },
                EventBody::LeaseAcquired {
                    work_item,
                    token,
                    expires_at,
                    note,
                } => Self::LeaseAcquired {
                    work_item: work_item.clone(),
                    token: *token,
                    expires_at: *expires_at,
                    note: note.clone(),
                },
                EventBody::LeaseRenewed {
                    work_item,
                    token,
                    expires_at,
                } => Self::LeaseRenewed {
                    work_item: work_item.clone(),
                    token: *token,
                    expires_at: *expires_at,
                },
                EventBody::LeaseReleased { work_item, token } => Self::LeaseReleased {
                    work_item: work_item.clone(),
                    token: *token,
                },
                EventBody::ClaimForgotten {
                    claim,
                    reason,
                    purged_blobs,
                } => Self::ClaimForgotten {
                    claim: *claim,
                    reason: reason.clone(),
                    purged_blobs: purged_blobs.clone(),
                },
            }
        }
    }

    impl WireEventBody {
        fn into_body(self) -> Result<EventBody, String> {
            Ok(match self {
                Self::ClaimAsserted {
                    claim,
                    supersedes,
                    data,
                } => EventBody::ClaimAsserted {
                    claim,
                    supersedes,
                    data: Box::new(data.into_claim_data()?),
                },
                Self::ClaimInvalidated { claim, at, reason } => {
                    EventBody::ClaimInvalidated { claim, at, reason }
                }
                Self::ClaimRetracted {
                    claim,
                    reason,
                    note,
                    corrected_by,
                } => EventBody::ClaimRetracted {
                    claim,
                    reason,
                    note,
                    corrected_by,
                },
                Self::EdgeAdded {
                    from,
                    to,
                    kind,
                    note,
                    evidence,
                } => EventBody::EdgeAdded {
                    from,
                    to,
                    kind,
                    note,
                    evidence,
                },
                Self::FeedbackRecorded {
                    claim,
                    outcome,
                    note,
                    evidence,
                } => EventBody::FeedbackRecorded {
                    claim,
                    outcome,
                    note,
                    evidence,
                },
                Self::Recovered { truncated_bytes } => EventBody::Recovered { truncated_bytes },
                Self::ArtifactStored {
                    hash,
                    size,
                    media_type,
                    name,
                    origin,
                } => EventBody::ArtifactStored {
                    hash,
                    size,
                    media_type,
                    name,
                    origin,
                },
                Self::LeaseAcquired {
                    work_item,
                    token,
                    expires_at,
                    note,
                } => EventBody::LeaseAcquired {
                    work_item,
                    token,
                    expires_at,
                    note,
                },
                Self::LeaseRenewed {
                    work_item,
                    token,
                    expires_at,
                } => EventBody::LeaseRenewed {
                    work_item,
                    token,
                    expires_at,
                },
                Self::LeaseReleased { work_item, token } => {
                    EventBody::LeaseReleased { work_item, token }
                }
                Self::ClaimForgotten {
                    claim,
                    reason,
                    purged_blobs,
                } => EventBody::ClaimForgotten {
                    claim,
                    reason,
                    purged_blobs,
                },
            })
        }
    }

    impl From<&ClaimData> for WireClaimData {
        fn from(data: &ClaimData) -> Self {
            let payload_json = serde_json::to_vec(&data.payload).unwrap_or_else(|_| b"{}".to_vec());
            Self {
                kind: data.kind,
                statement: data.statement.clone(),
                scope: data.scope.clone(),
                about: data.about.clone(),
                payload_json,
                applies_when: data
                    .applies_when
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
                valid_from: data.valid_from,
                valid_to: data.valid_to,
                review_by: data.review_by,
                provenance: WireProvenance::from(&data.provenance),
            }
        }
    }

    impl WireClaimData {
        fn into_claim_data(self) -> Result<ClaimData, String> {
            let payload: serde_json::Map<String, serde_json::Value> =
                if self.payload_json.is_empty() {
                    serde_json::Map::new()
                } else {
                    serde_json::from_slice(&self.payload_json)
                        .map_err(|e| format!("postcard claim payload json: {e}"))?
                };
            Ok(ClaimData {
                kind: self.kind,
                statement: self.statement,
                scope: self.scope,
                about: self.about,
                payload,
                applies_when: self.applies_when.into_iter().collect::<BTreeMap<_, _>>(),
                valid_from: self.valid_from,
                valid_to: self.valid_to,
                review_by: self.review_by,
                provenance: self.provenance.into(),
            })
        }
    }

    impl From<&Provenance> for WireProvenance {
        fn from(p: &Provenance) -> Self {
            Self {
                method: p.method,
                origin: p.origin,
                evidence: p.evidence.clone(),
                citations: p.citations.iter().map(WireCitation::from).collect(),
                derived_from: p.derived_from.clone(),
            }
        }
    }

    impl From<WireProvenance> for Provenance {
        fn from(p: WireProvenance) -> Self {
            Self {
                method: p.method,
                origin: p.origin,
                evidence: p.evidence,
                citations: p.citations.into_iter().map(Into::into).collect(),
                derived_from: p.derived_from,
            }
        }
    }

    impl From<&Citation> for WireCitation {
        fn from(c: &Citation) -> Self {
            Self {
                path: c.path.clone(),
                repo: c.repo.clone(),
                commit: c.commit.clone(),
                start_line: c.start_line,
                end_line: c.end_line,
                content_hash: c.content_hash.clone(),
            }
        }
    }

    impl From<WireCitation> for Citation {
        fn from(c: WireCitation) -> Self {
            Self {
                path: c.path,
                repo: c.repo,
                commit: c.commit,
                start_line: c.start_line,
                end_line: c.end_line,
                content_hash: c.content_hash,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hivemind_core::{
        Actor, ClaimData, ClaimId, ClaimKind, EVENT_VERSION, Event, EventBody, EventId, Scope,
        Timestamp,
    };
    use uuid::Uuid;

    fn sample_event() -> Event {
        let claim = ClaimId::from_uuid(Uuid::from_u128(0x1111_2222_3333_4444_5555_6666_7777_8888));
        let mut data = ClaimData::new(
            ClaimKind::Fact,
            "format freeze sample claim",
            Scope::Project("hivemind".into()),
        );
        data.about = vec!["file:log.rs".into()];
        data.payload.insert("k".into(), serde_json::json!("v"));
        Event {
            v: EVENT_VERSION,
            id: EventId::from_uuid(Uuid::from_u128(0xaaaa_bbbb_cccc_dddd_eeee_ffff_0000_1111)),
            recorded_at: Timestamp::from_millis(1_736_000_000_000),
            actor: Actor::agent("format-freeze"),
            idempotency_key: Some("freeze-1".into()),
            body: EventBody::ClaimAsserted {
                claim,
                supersedes: Vec::new(),
                data: Box::new(data),
            },
        }
    }

    #[test]
    fn json_v1_roundtrips_and_tags_first_byte() {
        let event = sample_event();
        let bytes = encode(&event).unwrap();
        assert_eq!(bytes[0], JSON_V1);
        assert_eq!(decode(0, &bytes).unwrap(), event);
    }

    #[test]
    fn postcard_v1_roundtrips_and_tags_first_byte() {
        let event = sample_event();
        let bytes = encode_with(EventCodec::PostcardV1, &event).unwrap();
        assert_eq!(bytes[0], POSTCARD_V1);
        assert_eq!(peek_codec(&bytes), Some(POSTCARD_V1));
        assert_eq!(decode(0, &bytes).unwrap(), event);
        assert!(bytes.len() < encode(&event).unwrap().len());
    }

    #[test]
    fn rejects_empty_unknown_and_bad_json() {
        assert!(matches!(
            decode(7, &[]),
            Err(Error::Decode { offset: 7, .. })
        ));
        let err = decode(3, &[9, b'{', b'}']).unwrap_err();
        match err {
            Error::Decode { offset: 3, reason } => assert!(reason.contains("unknown codec 9")),
            other => panic!("{other}"),
        }
        let err = decode(1, &[JSON_V1, b'{']).unwrap_err();
        assert!(matches!(err, Error::Decode { offset: 1, .. }));
    }

    #[test]
    fn encoding_is_deterministic() {
        let event = sample_event();
        assert_eq!(encode(&event).unwrap(), encode(&event).unwrap());
        assert_eq!(
            encode_with(EventCodec::PostcardV1, &event).unwrap(),
            encode_with(EventCodec::PostcardV1, &event).unwrap()
        );
    }

    #[test]
    fn event_codec_parse_names() {
        assert_eq!(EventCodec::parse("json").unwrap(), EventCodec::JsonV1);
        assert_eq!(
            EventCodec::parse("POSTCARD").unwrap(),
            EventCodec::PostcardV1
        );
        assert_eq!(EventCodec::parse("2").unwrap(), EventCodec::PostcardV1);
        assert!(EventCodec::parse("cbor").is_err());
    }
}
