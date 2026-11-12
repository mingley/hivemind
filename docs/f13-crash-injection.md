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

**Status:** LazyFS is **not** wired into `.github/workflows/ci.yml`. This is
an accepted residual risk for the first tagged release: process-crash and
checksum/torn-tail coverage ship in CI; true power-loss injection does not.

## Remaining crash-injection work (post-tag / P1)

Track under F13 until closed; none of these silently change the on-disk
format (see format freeze below).

1. **LazyFS on Linux CI** — self-hosted or containerized runner with FUSE;
   script that mounts LazyFS, runs mixed writes, injects “crash” (drop cache),
   reopens the store, asserts zero lost acks.
2. **Higher `kill -9` volume** — F13 asks for ~1,000 cycles; CI keeps 12 for
   runtime. Add a nightly / manual job that raises the loop count.
3. **Simulated filesystem** — evaluate `turmoil-fs` (or an in-process fault
   injector behind the file ops used by `log` / `blob`) for seeded crash
   schedules and torn writes at 512 B / 4 KiB boundaries.
4. **Disk-full and I/O-error injection beyond the unit-test hook** — extend
   fault injection to blob create/rename and directory fsync failures.
5. **ALICE-style / model-based store API tests** with crash/restart as an
   operation (complements frame-level properties).

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
