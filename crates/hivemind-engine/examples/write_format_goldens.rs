//! One-shot generator for format freeze fixtures. Not part of the public API.
use std::fs;
use std::path::PathBuf;

use hivemind_core::{
    Actor, ClaimData, ClaimId, ClaimKind, EVENT_VERSION, Event, EventBody, EventId, Scope,
    Timestamp,
};
use hivemind_engine::codec::{self, EventCodec};
use hivemind_engine::{encode_record, encode_segment_header};
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

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    fs::create_dir_all(root.join("log")).unwrap();
    fs::create_dir_all(root.join("codec")).unwrap();

    fs::write(
        root.join("log/segment_header_base0.bin"),
        encode_segment_header(0),
    )
    .unwrap();
    fs::write(
        root.join("log/segment_header_base41823.bin"),
        encode_segment_header(41823),
    )
    .unwrap();

    let events = vec![b"alpha".to_vec(), b"bravo".to_vec()];
    let record = encode_record(0, &events).unwrap();
    fs::write(root.join("log/commit_record_two_events.bin"), &record).unwrap();

    let payload = codec::encode(&sample_event()).unwrap();
    fs::write(root.join("codec/json_v1_claim_asserted.bin"), &payload).unwrap();

    let postcard = codec::encode_with(EventCodec::PostcardV1, &sample_event()).unwrap();
    fs::write(root.join("codec/postcard_v1_claim_asserted.bin"), &postcard).unwrap();

    let mut segment = Vec::new();
    segment.extend_from_slice(&encode_segment_header(0));
    segment.extend_from_slice(&encode_record(0, &[payload]).unwrap());
    fs::write(root.join("log/segment_one_event.bin"), &segment).unwrap();

    eprintln!("wrote goldens under {}", root.display());
}
