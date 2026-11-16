# F13 crash injection: CI coverage and remaining work

F13 ([07-falsification-experiments.md](07-falsification-experiments.md))
blocks every release: acknowledged writes must survive crashes, and
projections must rebuild consistently with the log.

## What CI runs today

| Layer | Where | What it proves |
|---|---|---|
| Process crash (`kill -9`) | `crates/hivemind/tests/crash.rs` | 12 mid-write SIGKILL cycles; every acknowledged claim survives reopen + `verify` |
| Torn-tail recovery | `crates/hivemind-engine` `log` unit tests | Truncation at every byte of the last record; bit-flips treated as torn; damage-with-later-valid-record refuses to open |
| Frame / codec property tests | `crates/hivemind-engine/tests/frame_props.rs` | Record round-trips; single-byte flips rejected; random bytes never panic the decoder or JSON_V1 codec |
| Golden format freeze | `crates/hivemind-engine/tests/format_freeze.rs` + `tests/fixtures/{log,codec}/` | Segment header, commit record, and JSON_V1 event bytes are pinned for format/codec version 1 |
| Disk-full path | `log` unit tests | `ENOSPC`-style refusal does not poison the log |

Together these are the **strongest in-CI substitute** for power-loss injection
on the current Ubuntu/macOS GitHub Actions matrix.

## LazyFS gap (power-loss / drop-unsynced)

[LazyFS](https://github.com/dsrhaslab/lazyfs) drops data that was written but
not explicitly flushed, which models power loss better than `kill -9` (process
crash with a live kernel page cache). It is a FUSE filesystem and needs:

- Linux hosts with FUSE available to CI runners
- A custom image or install step (C++ build of LazyFS)
- Privileges / device access that GitHub-hosted `ubuntu-latest` does not
  reliably provide without a self-hosted or specially imaged runner

**Status:** `.github/workflows/ci.yml` now builds pinned LazyFS commit
`3edad01aa936b81fed9bd64e47e725fc5ef6c977` and runs
`scripts/power_loss.sh`. The reusable CI gates release packaging too.
The test first proves that an unsynced negative-control file loses its data,
then checks 20 cycles of acknowledged claims/artifact blobs, segment rolls,
checkpoint + privacy erasure, cache loss, reopen and verify. CI fails if FUSE
cannot mount or if the negative control does not lose data; it never silently
skips this gate. Ordinary workspace tests mark only the mounted test ignored.

Local execution requires a Linux host that permits FUSE mounts. The managed
workspace used on 2026-11-16 refused mounts (`Operation not permitted`), so
runtime validation of this job must come from GitHub Actions.
LazyFS models lost **data cache**, not all possible hardware/filesystem
metadata failures. This is stronger coverage, not a blanket power-loss proof.

Erasure now atomically replaces a segment rather than overwriting a historical
record in place. All writers, backup and checkpoint creation serialize with
that operation. Startup resumes tombstoned incomplete erasures; blob and
snapshot cleanup failures prevent success acknowledgements.

## Extended process-crash testing

`HIVEMIND_CRASH_CYCLES=1000 cargo test --locked -p hivemind --test crash`
raises the default 12-cycle SIGKILL test for nightly/manual validation.
Future work: broader I/O fault schedules and metadata-aware power-failure
models; these do not replace the release gates above.

## Format freeze (first release tag)

On-disk versions pinned by goldens and explicit constants:

| Artifact | Constant | Value |
|---|---|---|
| Segment header | `log::SEGMENT_FORMAT` / magic `HMLOG\0\0\x01` | 1 |
| Event envelope | `codec::JSON_V1` | 1 |
| Event schema | `hivemind_core::EVENT_VERSION` | 1 |
| Store MANIFEST | `store::STORE_FORMAT` | 1 |

Regenerate fixtures only with an intentional bump:

```bash
cargo run -p hivemind-engine --example write_format_goldens
```

Changing any of these without new goldens and a documented migration must
fail CI.
