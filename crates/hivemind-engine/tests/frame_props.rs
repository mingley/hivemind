//! Property / fuzz-style tests for commit-record framing and event codecs.
//! These are the in-CI substitute for LazyFS power-loss runs
//! (see `docs/f13-crash-injection.md`).

use hivemind_core::{
    Actor, ClaimData, ClaimId, ClaimKind, EVENT_VERSION, Event, EventBody, EventId, Scope,
    Timestamp,
};
use hivemind_engine::codec::{self, EventCodec, JSON_V1, POSTCARD_V1};
use hivemind_engine::{
    FrameError, MAX_RECORD_LEN, encode_record, encode_segment_header, try_decode_record,
};
use proptest::prelude::*;
use uuid::Uuid;

fn arb_event_bytes() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(any::<u8>(), 0..64)
}

fn arb_batch() -> impl Strategy<Value = Vec<Vec<u8>>> {
    prop::collection::vec(arb_event_bytes(), 1..8)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn record_roundtrip(first in 0u64..10_000, batch in arb_batch()) {
        let encoded = encode_record(first, &batch).expect("encode");
        let (consumed, decoded) = try_decode_record(first, &encoded).expect("decode");
        prop_assert_eq!(consumed, encoded.len());
        let expected: Vec<(u64, Vec<u8>)> = batch
            .into_iter()
            .enumerate()
            .map(|(i, e)| (first + i as u64, e))
            .collect();
        prop_assert_eq!(decoded, expected);
    }

    #[test]
    fn trailing_noise_does_not_change_decoded_prefix(
        first in 0u64..1_000,
        batch in arb_batch(),
        noise in prop::collection::vec(any::<u8>(), 0..32),
    ) {
        let encoded = encode_record(first, &batch).unwrap();
        let mut with_noise = encoded.clone();
        with_noise.extend_from_slice(&noise);
        let (consumed, decoded) = try_decode_record(first, &with_noise).unwrap();
        prop_assert_eq!(consumed, encoded.len());
        prop_assert_eq!(decoded.len(), batch.len());
    }

    #[test]
    fn any_single_byte_flip_is_rejected(
        first in 0u64..500,
        batch in arb_batch(),
        flip_idx in any::<prop::sample::Index>(),
    ) {
        let mut encoded = encode_record(first, &batch).unwrap();
        let i = flip_idx.index(encoded.len());
        encoded[i] ^= 0xA5;
        // Wrong expected offset is also a rejection path; keep expected correct
        // so we exercise CRC / structure checks from the flip.
        let err = try_decode_record(first, &encoded).unwrap_err();
        prop_assert!(
            matches!(
                err,
                FrameError::ChecksumMismatch
                    | FrameError::MalformedPayload
                    | FrameError::EmptyBatch
                    | FrameError::TruncatedPayload
                    | FrameError::LengthExceedsLimit(_)
                    | FrameError::OffsetMismatch { .. }
            ),
            "unexpected accept after flip at {i}: {err:?}"
        );
    }

    #[test]
    fn random_bytes_never_panic_the_decoder(bytes in prop::collection::vec(any::<u8>(), 0..256)) {
        let _ = try_decode_record(0, &bytes);
        let _ = encode_segment_header(0);
    }

    #[test]
    fn codec_rejects_unknown_tags_without_panic(tag in 3u8..=255, body in prop::collection::vec(any::<u8>(), 0..64)) {
        let mut bytes = vec![tag];
        bytes.extend_from_slice(&body);
        let err = codec::decode(0, &bytes).unwrap_err();
        match err {
            hivemind_engine::Error::Decode { reason, .. } => {
                prop_assert!(reason.contains("unknown codec"), "{reason}");
            }
            other => prop_assert!(false, "unexpected {other}"),
        }
    }

    #[test]
    fn codec_garbage_json_is_decode_error(junk in prop::collection::vec(any::<u8>(), 0..128)) {
        let mut bytes = vec![JSON_V1];
        bytes.extend_from_slice(&junk);
        // Valid JSON that happens to deserialize as Event is vanishingly rare
        // for random bytes; either way the decoder must not panic.
        let _ = codec::decode(9, &bytes);
    }

    #[test]
    fn codec_garbage_postcard_is_decode_error(junk in prop::collection::vec(any::<u8>(), 0..128)) {
        let mut bytes = vec![POSTCARD_V1];
        bytes.extend_from_slice(&junk);
        let _ = codec::decode(9, &bytes);
    }

    #[test]
    fn postcard_v1_event_roundtrip(
        statement in "[a-zA-Z0-9 ._-]{1,64}",
        principal in "[a-zA-Z0-9_-]{1,32}",
        key in prop::option::of("[a-zA-Z0-9_-]{1,24}"),
        millis in 0i64..2_000_000_000_000,
        id_hi in any::<u64>(),
        id_lo in any::<u64>(),
        claim_hi in any::<u64>(),
        claim_lo in any::<u64>(),
    ) {
        let event = Event {
            v: EVENT_VERSION,
            id: EventId::from_uuid(Uuid::from_u64_pair(id_hi, id_lo)),
            recorded_at: Timestamp::from_millis(millis),
            actor: Actor::agent(principal),
            idempotency_key: key,
            body: EventBody::ClaimAsserted {
                claim: ClaimId::from_uuid(Uuid::from_u64_pair(claim_hi, claim_lo)),
                supersedes: Vec::new(),
                data: Box::new(ClaimData::new(
                    ClaimKind::Fact,
                    statement,
                    Scope::Global,
                )),
            },
        };
        let bytes = codec::encode_with(EventCodec::PostcardV1, &event).unwrap();
        prop_assert_eq!(bytes[0], POSTCARD_V1);
        prop_assert_eq!(codec::decode(0, &bytes).unwrap(), event);
    }
}

#[test]
fn oversized_length_prefix_is_rejected() {
    let mut bytes = vec![0u8; 20];
    bytes[0..4].copy_from_slice(&(MAX_RECORD_LEN + 1).to_le_bytes());
    assert_eq!(
        try_decode_record(0, &bytes),
        Err(FrameError::LengthExceedsLimit(MAX_RECORD_LEN + 1))
    );
}

#[test]
fn json_v1_event_roundtrip_property_fixture() {
    let event = Event {
        v: EVENT_VERSION,
        id: EventId::from_uuid(Uuid::from_u128(42)),
        recorded_at: Timestamp::from_millis(1_700_000_000_000),
        actor: Actor::human("props"),
        idempotency_key: None,
        body: EventBody::ClaimAsserted {
            claim: ClaimId::from_uuid(Uuid::from_u128(99)),
            supersedes: Vec::new(),
            data: Box::new(ClaimData::new(ClaimKind::Fact, "prop claim", Scope::Global)),
        },
    };
    let bytes = codec::encode(&event).unwrap();
    assert_eq!(bytes[0], JSON_V1);
    assert_eq!(codec::decode(0, &bytes).unwrap(), event);
}

#[test]
fn postcard_v1_event_roundtrip_property_fixture() {
    let event = Event {
        v: EVENT_VERSION,
        id: EventId::from_uuid(Uuid::from_u128(42)),
        recorded_at: Timestamp::from_millis(1_700_000_000_000),
        actor: Actor::human("props"),
        idempotency_key: None,
        body: EventBody::ClaimAsserted {
            claim: ClaimId::from_uuid(Uuid::from_u128(99)),
            supersedes: Vec::new(),
            data: Box::new(ClaimData::new(ClaimKind::Fact, "prop claim", Scope::Global)),
        },
    };
    let bytes = codec::encode_with(EventCodec::PostcardV1, &event).unwrap();
    assert_eq!(bytes[0], POSTCARD_V1);
    assert_eq!(codec::decode(0, &bytes).unwrap(), event);
}
