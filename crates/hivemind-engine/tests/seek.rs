//! Sparse lookup must preserve feed boundaries, batches, rolls and old views.
use hivemind_engine::{Durability, log::Log};

#[test]
fn indexed_reads_match_full_scan_across_batches_rolls_reopen_and_erasure() {
    let dir = tempfile::tempdir().unwrap();
    let (mut log, _) = Log::open(dir.path(), Durability::NoFsync, 4096, |_, _| Ok(())).unwrap();
    for start in (0..660).step_by(3) {
        let events: Vec<_> = (start..start + 3)
            .map(|i| format!("event-{i:05}").into_bytes())
            .collect();
        log.append(&events).unwrap();
    }
    let old = log.view();
    log.append(&[b"later event".to_vec()]).unwrap();
    assert_eq!(old.end(), 660);
    assert!(old.read(660, 10).unwrap().is_empty());
    assert_eq!(old.read(655, 20).unwrap().len(), 5);
    log.rewrite_event(257, b"redacted").unwrap();
    let check = |view: hivemind_engine::log::LogView| {
        let all = view.read(0, usize::MAX).unwrap();
        for from in [0, 1, 126, 127, 128, 129, 255, 256, 257, 511, 659, 660, 700] {
            for limit in [0, 1, 3, 17, 1000] {
                let expected: Vec<_> = all
                    .iter()
                    .filter(|(offset, _)| *offset >= from)
                    .take(limit)
                    .cloned()
                    .collect();
                assert_eq!(
                    view.read(from, limit).unwrap(),
                    expected,
                    "from={from} limit={limit}"
                );
            }
        }
        assert!(view.read(257, 1).unwrap()[0].1.starts_with(b"redacted"));
    };
    check(log.view());
    drop(log);
    let (reopened, _) = Log::open(dir.path(), Durability::NoFsync, 4096, |_, _| Ok(())).unwrap();
    check(reopened.view());
}
