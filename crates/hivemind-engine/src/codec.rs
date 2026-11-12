//! Event encoding. The first byte names the codec, so the format can evolve
//! without rewriting history; readers keep every codec they ever wrote.
//!
//! [`JSON_V1`] is the only codec shipped in the first release. Golden fixtures
//! under `tests/fixtures/codec/` freeze its on-disk bytes; bumping this tag or
//! changing JSON field layout requires new fixtures and an explicit migration.

use hivemind_core::Event;

use crate::error::{Error, Result};

/// serde_json encoding of [`Event`]. Readable with standard tools, and struct
/// fields serialize in declaration order, so encodings are deterministic.
pub const JSON_V1: u8 = 1;

pub fn encode(event: &Event) -> Result<Vec<u8>> {
    let mut bytes = vec![JSON_V1];
    serde_json::to_writer(&mut bytes, event).map_err(|e| Error::Invalid(e.to_string()))?;
    Ok(bytes)
}

pub fn decode(offset: u64, bytes: &[u8]) -> Result<Event> {
    match bytes.split_first() {
        Some((&JSON_V1, json)) => serde_json::from_slice(json).map_err(|e| Error::Decode {
            offset,
            reason: e.to_string(),
        }),
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
    fn rejects_empty_unknown_and_bad_json() {
        assert!(matches!(
            decode(7, &[]),
            Err(Error::Decode { offset: 7, .. })
        ));
        let err = decode(3, &[2, b'{', b'}']).unwrap_err();
        match err {
            Error::Decode { offset: 3, reason } => assert!(reason.contains("unknown codec 2")),
            other => panic!("{other}"),
        }
        let err = decode(1, &[JSON_V1, b'{']).unwrap_err();
        assert!(matches!(err, Error::Decode { offset: 1, .. }));
    }

    #[test]
    fn encoding_is_deterministic() {
        let event = sample_event();
        assert_eq!(encode(&event).unwrap(), encode(&event).unwrap());
    }
}
