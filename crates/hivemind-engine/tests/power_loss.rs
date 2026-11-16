//! Runs only on the mounted LazyFS CI job. The negative control proves the
//! injector actually discards unsynced bytes before durability is tested.
use hivemind_core::{Actor, ClaimData, ClaimKind, EvidenceRef, Origin};
use hivemind_engine::{Durability, Options, Remember, Store};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;

fn lose_unsynced_data() {
    let mut completed = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(std::env::var_os("HIVEMIND_LAZYFS_COMPLETED").expect("completion FIFO"))
        .unwrap();
    let mut command = std::fs::OpenOptions::new()
        .write(true)
        .open(std::env::var_os("HIVEMIND_LAZYFS_COMMAND").expect("command FIFO"))
        .unwrap();
    writeln!(command, "lazyfs::clear-cache").unwrap();
    let mut line = String::new();
    BufReader::new(&mut completed).read_line(&mut line).unwrap();
    assert!(line.contains("finished::clear-cache"), "{line}");
}

#[test]
#[ignore = "requires mounted LazyFS; scripts/power_loss.sh"]
fn acknowledged_claims_blobs_and_erasure_survive_power_loss() {
    let root = PathBuf::from(std::env::var_os("HIVEMIND_POWER_LOSS_ROOT").expect("LazyFS root"));
    let control = root.join("negative-control");
    std::fs::write(&control, b"intentionally not fsynced").unwrap();
    lose_unsynced_data();
    assert_ne!(
        std::fs::read(&control).unwrap_or_default(),
        b"intentionally not fsynced",
        "fault injector did not drop unsynced data"
    );
    let actor = Actor::agent("power-loss-test");
    let path = root.join("store");
    let mut acked = Vec::new();
    let mut forgotten = Vec::new();
    for round in 0..20 {
        let store = Store::open(
            &path,
            Options {
                durability: Durability::Fsync,
                segment_bytes: 4096,
                ..Options::default()
            },
        )
        .unwrap();
        let bytes = format!("durable transcript round {round}").into_bytes();
        let (hash, _) = store
            .put_artifact(&actor, &bytes, None, None, Origin::Agent)
            .unwrap();
        let mut data = ClaimData::new(
            ClaimKind::Fact,
            format!("durable memory round {round}"),
            "project:crash".parse().unwrap(),
        );
        data.provenance.evidence.push(EvidenceRef::Artifact(hash));
        let id = store
            .remember(&actor, Remember::new(data))
            .unwrap()
            .claim
            .unwrap();
        if round % 3 == 0 {
            store.checkpoint().unwrap();
            store
                .forget(&actor, id, "power-loss erasure", None)
                .unwrap();
            forgotten.push(id);
        } else {
            acked.push((id, hash, bytes));
        }
        drop(store);
        lose_unsynced_data();
        let reopened = Store::open(&path, Options::default()).unwrap();
        for (id, hash, bytes) in &acked {
            assert!(
                reopened.get(id, None).is_some(),
                "lost acknowledged claim {id}"
            );
            assert_eq!(&reopened.artifact(hash).unwrap(), bytes);
        }
        for id in &forgotten {
            assert!(reopened.get(id, None).is_none());
        }
        assert!(reopened.verify().unwrap().is_ok());
    }
}
