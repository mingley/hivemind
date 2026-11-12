//! Golden fixtures that freeze segment-header, commit-record, and event
//! codec bytes for [`SEGMENT_FORMAT`] / [`JSON_V1`] / [`POSTCARD_V1`]. A
//! silent format change fails these tests before the first release tag.

use std::fs;
use std::path::PathBuf;

use hivemind_core::{
    Actor, ClaimData, ClaimId, ClaimKind, EVENT_VERSION, Event, EventBody, EventId, Scope,
    Timestamp,
};
use hivemind_engine::codec::{self, EventCodec, JSON_V1, POSTCARD_V1};
use hivemind_engine::store::STORE_FORMAT;
use hivemind_engine::{
    SEGMENT_FORMAT, SEGMENT_MAGIC, encode_record, encode_segment_header, try_decode_record,
};
use uuid::Uuid;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn read_fixture(rel: &str) -> Vec<u8> {
    let path = fixtures_dir().join(rel);
    fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

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
fn release_format_versions_are_pinned() {
    assert_eq!(
        SEGMENT_FORMAT, 1,
        "segment header format must not change silently"
    );
    assert_eq!(JSON_V1, 1, "event codec id must not change silently");
    assert_eq!(POSTCARD_V1, 2, "postcard codec id must not change silently");
    assert_eq!(
        EVENT_VERSION, 1,
        "event schema version must not change silently"
    );
    assert_eq!(
        STORE_FORMAT, 1,
        "MANIFEST store format must not change silently"
    );
    assert_eq!(&SEGMENT_MAGIC, b"HMLOG\0\0\x01");
}

#[test]
fn segment_header_golden_matches_encode() {
    let got = encode_segment_header(0);
    let want = read_fixture("log/segment_header_base0.bin");
    assert_eq!(
        got.as_slice(),
        want.as_slice(),
        "segment header bytes drifted; update fixtures only with an intentional format bump"
    );
    let got_base = encode_segment_header(41823);
    let want_base = read_fixture("log/segment_header_base41823.bin");
    assert_eq!(got_base.as_slice(), want_base.as_slice());
}

#[test]
fn commit_record_golden_matches_encode() {
    let events = vec![b"alpha".to_vec(), b"bravo".to_vec()];
    let got = encode_record(0, &events).unwrap();
    let want = read_fixture("log/commit_record_two_events.bin");
    assert_eq!(got, want);
    let (consumed, decoded) = try_decode_record(0, &got).unwrap();
    assert_eq!(consumed, got.len());
    assert_eq!(
        decoded,
        vec![(0, b"alpha".to_vec()), (1, b"bravo".to_vec())]
    );
}

#[test]
fn codec_json_v1_golden_matches_encode() {
    let event = sample_event();
    let got = codec::encode(&event).unwrap();
    let want = read_fixture("codec/json_v1_claim_asserted.bin");
    assert_eq!(
        got, want,
        "JSON_V1 event bytes drifted; regenerate fixtures only with an intentional codec bump"
    );
    assert_eq!(got[0], JSON_V1);
    assert_eq!(codec::decode(0, &got).unwrap(), event);
}

#[test]
fn codec_postcard_v1_golden_matches_encode() {
    let event = sample_event();
    let got = codec::encode_with(EventCodec::PostcardV1, &event).unwrap();
    let want = read_fixture("codec/postcard_v1_claim_asserted.bin");
    assert_eq!(
        got, want,
        "POSTCARD_V1 event bytes drifted; regenerate fixtures only with an intentional codec bump"
    );
    assert_eq!(got[0], POSTCARD_V1);
    assert_eq!(codec::decode(0, &got).unwrap(), event);
    assert!(
        got.len() < codec::encode(&event).unwrap().len(),
        "postcard should be more compact than JSON for the freeze sample"
    );
}

#[test]
fn segment_plus_record_golden_replays() {
    let header = encode_segment_header(0);
    let payload = codec::encode(&sample_event()).unwrap();
    let record = encode_record(0, std::slice::from_ref(&payload)).unwrap();
    let mut segment = Vec::new();
    segment.extend_from_slice(&header);
    segment.extend_from_slice(&record);
    let want = read_fixture("log/segment_one_event.bin");
    assert_eq!(segment, want);

    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("00000000000000000000.hlog");
    fs::write(&path, &segment).unwrap();
    let mut seen = Vec::new();
    let (log, recovery) = hivemind_engine::log::Log::open(
        tmp.path(),
        hivemind_engine::Durability::NoFsync,
        hivemind_engine::log::DEFAULT_SEGMENT_BYTES,
        |offset, bytes| {
            seen.push((offset, bytes.to_vec()));
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(recovery.events, 1);
    assert_eq!(recovery.truncated_bytes, 0);
    assert_eq!(seen, vec![(0, payload)]);
    assert_eq!(log.next_offset(), 1);
}
