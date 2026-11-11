# Research 09: Engine design techniques for an all-Rust storage engine

> Research thread report, checked 2026-11-10. Produced by a research agent from public sources and saved verbatim (delivered in three parts and joined here). Evidence labels: **[V]** verified in a primary source, **[C]** claim, **[I]** independent evidence, **[Inf]** inference, **[?]** unknown. Synthesis and decisions live in [`docs/`](../docs/).

## Executive summary

- **Engine shape for v0:** an event log that is only appended to, split into segment files and indexed by a monotonic sequence number. Large artifacts go in a separate store keyed by their BLAKE3 hash. All derived state and indexes live in memory and are rebuilt by replaying the log. Readers get a consistent snapshot through `arc-swap`.
  - This combines Bitcask, WiscKey's separation of large values, Kafka-style segments and Lucene-style near-real-time readers. A general LSM tree or B+tree for the main data isn't needed, because the log is already ordered by sequence number.
  - v1 adds checkpoints, plus index segments on disk that are never modified after writing and are merged in tiers. v2 adds replication by shipping segments, partitioning, and an approximate vector index only where measurements show it's needed.
- **Durability, verified in primary sources:**
  - **Linux:** acknowledge a write only after `fdatasync`. After creating or renaming a file, also fsync its directory. Applications most often lose data by skipping that directory flush.
  - **macOS:** Rust's `sync_data` and `sync_all` both already call `F_FULLFSYNC`.
  - **Handling fsync errors:**
    - Treat any fsync error as fatal and restart, as PostgreSQL does since the 2018 "fsyncgate" problem. A failed fsync can leave pages marked clean even though they never reached disk.
    - On restart, cut off a torn final record. Refuse to start if a bad record has valid records after it, because that means corruption rather than a crash mid-write.
  - **Record framing:** use CRC32C. Zero-length records and a zero record type must be invalid. The checksum should cover the sequence number and a segment ID or epoch.
  - **Blob writes:** `tempfile::persist` syncs neither the file nor its directory, so it can't be used on its own.
- **Don't use io_uring, `O_DIRECT` or mmap for v0.** io_uring doesn't exist on macOS, and the Rust io_uring runtimes have been quiet since 2024. Google restricted io_uring and Docker's default security profile blocks it. Crotty, Leis and Pavlo (CIDR 2022) argue against mmap for anything that writes.
- **Testing:**
  - Put every file operation behind one interface and run seeded crash simulations that drop unsynced data and tear writes.
  - The new `turmoil-fs` crate (0.1.0, October 2026) models exactly this, but it's young.
  - Add LazyFS crash runs on Linux CI, kill -9 loops (these test process crashes only, not power loss), state-machine property tests, fuzzing of the record decoder, and frozen golden files for every format version.
- **Text search:**
  - BM25 across multiple fields, with k1 = 1.2 and b = 0.75 (Lucene and Tantivy defaults).
  - A code-aware tokenizer that splits camelCase and snake_case identifiers but keeps the original form, and handles version strings and paths.
  - Simple exhaustive scoring is fine for v0; add MaxScore pruning in v1.
  - Main memory risk: code text produces a huge vocabulary. Cap token types and move to compressed blocks of 128 documents on disk by about 1M items.
- **Vectors:**
  - v0 is an exact scan over 8-bit codes with full-precision rescoring; then 1-bit or RaBitQ codes for about 0.2–2M vectors per scope.
  - Exact scans give perfect recall under any filter, and our filters are heavy.
  - When an approximate index is needed, evaluate IVF+RaBitQ and Microsoft's Rust DiskANN library (MIT; plug in our store through its `DataProvider` trait).
  - Avoid SimSIMD, now renamed NumKong: it is C code built with CMake.
- **Serialization:**
  - postcard inside a versioned envelope, with upcasting on read.
  - Hash and sign the exact stored bytes; never re-encode to verify.
  - Avoid rkyv (no schema evolution, five security advisories in 2026) and bincode (officially unmaintained). The CBOR crates' canonical-encoding support is weak.
- **Crates:** use crc32c, blake3 (enable its `pure` feature for Rust-only builds), roaring, bitpacking, postcard, serde_json, arc-swap, tokio 1.54.1 or later, half, bytemuck, uuid (v7), and std `File::try_lock` (stable since Rust 1.89).

Target path: `/Users/mingley/dev/hivemind/research/09-engine-design-techniques.md`. I couldn't write files in this session, so the report is delivered as text. No files were modified.

---

## Research 09: Engine design techniques for hivemind's own Rust storage engine

> Research thread report, checked 2026-11-10, from public sources only. Labels: **[V]** verified in a primary source (cited), **[C]** project or vendor claim, **[I]** independent evidence, **[Inf]** inference, **[?]** unknown. Citations [Sn] point to §7. Code citations use `org/repo:path:lines` (default branch, checked 2026-11-10).

## 1. Summary (decisions this research implies)

1. **The v0 engine is "Bitcask with segments".** It has:
   - An append-only log of event frames in segment files, with a dense u64 sequence number as the primary key.
   - A sparse sequence→offset and time→sequence index per segment, Kafka style.
   - In-memory projections and indexes rebuilt by replay.
   - A separate content-addressed blob store for artifacts (WiscKey-style separation).

   No LSM tree or B+tree is needed for the main data, because the log is already sorted by key. **[Inf]** from [S1][S2][S9][S11].
2. **One writer thread and many readers on published snapshots.**
   - The writer collects requests, makes one `write` and one fdatasync per batch, applies the batch to projections, publishes a new snapshot with `ArcSwap`, and then acknowledges.
   - Readers never take locks and never see data that isn't yet durable. **[Inf]** (the pattern matches Kafka's copy-on-write segment list [S9] and Lucene's near-real-time readers [S12]).
3. **Acknowledge only after the data is durable.**
   - Linux: `fdatasync` on the segment, plus fsync of the directory after creating or renaming a file.
   - macOS: Rust's `sync_data` and `sync_all` both call `F_FULLFSYNC` **[V]** [S21]. Apple says plain `fsync` doesn't flush the drive cache **[V]** [S22].
4. **Any fsync or write error is fatal.** The process stops and restarts, then recovers from what is actually on disk. Never retry fsync. **[V]** basis: [S18][S19][S20].
5. **Recovery tells a torn tail from corruption.**
   - Cut off a bad final record only if no valid record follows it.
   - Otherwise refuse to start. Silently cutting off the log would drop acknowledged data.

   **[Inf]** from Kafka's recovery notes [S9] and TigerBeetle's storage fault model [S14].
6. **Record framing:**
   - CRC32C, masked as LevelDB does, covering the length, a non-zero record type, the sequence number and a segment epoch.
   - A record type of 0 and a length of 0 are invalid, so zero-filled or preallocated space can't parse as records.

   **[V]** precedents: [S24][S25].
7. **Blob store:** write to a temp file, fsync it, rename it to its hash path, then fsync the target directory. Clean the temp directory on startup and check the BLAKE3 hash when reading. `tempfile::persist` syncs neither file nor directory **[V]** [S95].
8. **Buffered I/O; no io_uring, `O_DIRECT` or mmap in v0.** **[V]** inputs: [S27][S28][S29][S30][S79]. Revisit `O_DIRECT` and our own page cache only for Linux servers in v2, as TigerBeetle does [S14].
9. **The testing pyramid is a simulated file system with crashes and torn writes**, plus LazyFS on Linux CI, kill -9 loops, model-based property tests, fuzzing of the decoder and recovery scanner, loom or shuttle for concurrency, and golden fixtures for every format version. **[V]** tools: [S32][S38][S39][S40][S42][S43].
10. **BM25 search:**
    - Multiple fields (BM25F) with k1 = 1.2 and b = 0.75 **[V]** [S12][S13][S55].
    - Splits camelCase, snake_case, digits and paths while keeping the original token, and handles versions **[V]** semantics [S59].
    - Exhaustive scoring with a dense accumulator in v0; MaxScore in v1 (Lucene now runs disjunctions through `MaxScoreBulkScorer` **[V]** [S12]).
11. **Vectors:**
    - v0 is an exact, filter-aware scan over 8-bit codes with full-precision rescoring.
    - Next comes 1-bit or RaBitQ prefiltering [S60][S61], and an approximate index only past about 1–2M vectors per scope or for server query rates.
    - Candidates are IVF+RaBitQ [S62] or DiskANN3 through `DataProvider` [S73]. **[Inf]** plus [V] inputs.
12. **Filters and graph:** roaring bitmaps over dense per-store document IDs for scope, kind and status. The graph is in-memory out- and in-adjacency (a compressed sparse row array plus recent changes). "As of" queries use per-entity version chains keyed by log sequence number, so the text and vector indexes need no history. **[Inf]**
13. **Serialization:**
    - postcard (wire format stable since v1 **[V]** [S85]) inside a versioned envelope, with upcasters chained from version to version.
    - Hash and sign the exact stored bytes, because postcard doesn't enforce a canonical encoding **[V]** [S85].
    - Deterministic CBOR (RFC 8949 §4.2.1) only if cross-language canonical bytes are ever needed [S84].
14. **Avoid these crates:** rkyv (no schema evolution [S86] and five 2026 advisories [S88]), bincode (unmaintained, RUSTSEC-2025-0141 [S88]), `im` (unmaintained; use `imbl`), SimSIMD/NumKong (C built with CMake [S75]), and fs2 (last release 2018).
15. **Memory is the main risk at 1M items on a laptop.** At 1M items, vocabulary, postings, vector codes and projections together exceed a lean memory budget. v0 should target about 200k items or fewer held in memory per store. v1 must move indexes to on-disk segments read through the page cache. **[Inf]** (estimates in §2.4 and §2.5).

---

## 2. Findings by topic

### 2.1 Log-structured storage designs and fit

| Design | Core idea (verified) | Fit for hivemind | What we take |
|---|---|---|---|
| **Bitcask** (Basho, 2010) | Append-only data files. An in-memory "keydir" hash table maps each key to (file, offset, size). A read needs at most one seek. Merge rewrites the files that are no longer written to, keeping only live values, and writes "hint files" holding positions without values. Deletes are tombstones. **[V]** [S1] | Excellent for v0. Our "keys" are dense sequence numbers, so the keydir collapses into a sparse index. The keydir must fit in RAM. | Log + in-memory index; hint files become our checkpoints; merge is used only for erasure or compaction. |
| **WiscKey** (FAST 2016) | Keeps keys in the LSM tree and values in a separate log to cut I/O amplification. Reports 2.5–111× faster loading than LevelDB. **[V]** [S2] | The blob store is our version of value separation. Garbage collection becomes reachability-based. | Large artifacts never go in the event log. |
| **FASTER / HybridLog** (SIGMOD 2018) | A concurrent hash index plus a "hybrid log" spanning memory and storage. Hot records are updated in place in a mutable region. Reported up to 160M ops/s. **[V]** [S3] The repo now says it is "no longer actively maintained" and points to Garnet/Tsavorite. **[V]** [S4] | Poor fit. We have immutable events and modest write rates; in-place updates aren't wanted. | Only the idea of a "read-only / durable address" watermark. |
| **LSM design space** | O'Neil et al. 1996 [S5]. Monkey shows stores mis-tune merge policy, buffer size and per-level Bloom filter false-positive rates together. **[V]** [S6] Dostoevsky shows most merges barely reduce lookup cost or space amplification and proposes adaptive merging. **[V]** [S7] | Not needed for main data. Relevant later for random-key indexes too big for RAM (idempotency keys, hash→sequence at 10M+). | If we build sorted runs: tiered merging at small levels, leveled at the largest; more Bloom bits at small levels. **[Inf]** |
| **LMDB** | Copy-on-write B+tree in a memory map. One writer at a time; versioned so readers take no locks. "No special recovery procedures" after a crash. Long-lived read transactions "prevent reuse of pages … the database can grow quickly". **[V]** [S8] Two meta pages (`NUM_METAS 2`). **[V]** [S8] | A good model for single writer + many readers, but mmap-based (§2.7). | A/B (two-slot) manifest for atomic root updates; watch for readers that hold old snapshots too long. |
| **Kafka segments** | Segments named by base offset. Sparse offset→file-position index files are preallocated, memory-mapped, "no attempt is made to checksum… rebuilt" after a crash. **[V]** [S11] A time index maps timestamp→offset with monotonic timestamps. **[V]** [S11] Batches use CRC-32C. **[V]** [S10] Recovery scans the newest segment and truncates at the first invalid entry. It notes a file "can gain nonsense data if the inode is updated with a new size but a crash occurs before the block … is written". **[V]** [S9] A copy-on-write segment list gives readers stable snapshots. **[V]** [S9] | Very good fit for log layout, sequence and time lookup, retention and replication by shipping segments. | Directly: segment naming, sparse seq→offset and time→seq indexes, copy-on-write segment list. |
| **Lucene / Tantivy** | Indexes are sets of immutable segments. Deletes go into per-segment "alive bitset" files. Commit writes segments and then atomically updates `meta.json` (Tantivy). **[V]** [S13] Near-real-time readers see uncommitted writer changes. **[V]** [S12] Lucene `TieredMergePolicy` defaults: `segsPerTier=8`, floor 16 MB, max merged segment 5 GB, `deletesPctAllowed=20`. **[V]** [S12] Tantivy `LogMergePolicy` defaults: min 8 segments per merge, minimum layer 10k docs, max 10M docs before merge. **[V]** [S13] | Exactly the model for our derived text and vector indexes from v1. | Immutable index segments, alive bitsets, tiered merges, a manifest naming the segments and the indexed-up-to sequence number. |
| **TigerBeetle** | An LSM "forest" of "grooves" (groups of trees). Incremental compaction runs in "beats"/"bars" after each commit. Growth factor typically 8, 7 levels. **[V]** [S14] All memory is allocated statically at startup. **[V]** [S14] Assumes the disk will fail: everything checksummed and hash-chained, `O_DIRECT`, its own page cache. **[V]** [S14] A block's checksum is stored outside the block (in its parent) to catch misdirected writes. **[V]** [S14] Deterministic simulator (VOPR) stubs clock, network and disk. **[V]** [S14][S35] | Philosophy fits; static allocation and `O_DIRECT` are overkill for v0. | Explicit storage-fault model; checksums stored in the parent (manifest stores segment hashes); bounded memory budgets; deterministic simulation. |
| **Datomic / XTDB** | Datomic is accumulate-only: retraction is new data; reads go against an immutable database value; as-of views; four sort-order indexes (EAVT, AEVT, AVET, VAET). **[V]** [S15] "Excision" deletes physically and is deliberately lossy. **[V]** [S15] XTDB 2: bitemporal, columnar on Apache Arrow, designed for object storage, "embraces a log as the central point of coordination". **[V]** [S16] `ERASE` removes data "even [from] queries as of a previous system-time". **[V]** [S16] | Matches our event model: supersession and retraction are new events, as-of uses system time, erasure is a rare physical purge. | EAVT-style projections for entities and edges; erasure as an explicit, auditable, lossy operation. |

**Recommended v0 shape [Inf]:** a single store directory per (tenant, project).

```
<store>/
  LOCK                                  # std File::try_lock (flock), single-writer guard
  MANIFEST.a / MANIFEST.b               # A/B slots with a generation counter and a checksum
  log/<base_seq:020>.seg                # active + sealed segments (roll at 64–256 MiB)
  log/<base_seq:020>.sidx               # sparse seq→offset and time→seq index (rebuildable)
  blobs/<ab>/<cd>/<blake3-hex>          # content-addressed artifacts
  blobs/tmp/                            # in-flight blob writes; emptied on startup
  (v1) checkpoints/, index/<segment-id>/
```

**Growth path:**
- **v1:** projection checkpoints written as files plus the log sequence they cover, so startup loads the checkpoint and replays only the suffix. Immutable on-disk text and vector index segments with tiered merges. A sealed-segment footer and BLAKE3 hash recorded in the manifest.
- **v2:** shipping sealed segments plus the active tail to followers; partitioning by (tenant, project); sorted-run indexes (SSTable style) for random keys too big for RAM; approximate vector index; optional `O_DIRECT` and our own cache on Linux servers.

### 2.2 Durability and crash consistency

**What the evidence says:**
- **ALICE (OSDI 2014).** How file systems persist updates "vary widely". For example, ext3 ordered mode persists an append to file A before a later rename of file B, but ext4 ordered mode doesn't unless a special option is set. The authors found 60 vulnerabilities in 11 applications. For 7 of the 11, expected durability wasn't met "often due to directory operations not being flushed". 10 of 11 expected file-system updates to be atomic, which is reasonable for single-sector writes and renames but less so for appends. **[V]** [S17]
- **"Can Applications Recover from fsync Failures?" (ATC 2020).** On ext4, XFS and Btrfs, after an fsync failure "pages are always marked clean". Of PostgreSQL, LMDB, LevelDB, SQLite and Redis, none handled fsync failures sufficiently; outcomes included data loss and corruption. **[V]** [S18]
- **fsyncgate (2018).** When buffered writeback fails, kernels typically discard the data and mark pages clean. Errors that happened before a file descriptor was opened may not be reported. **[V]** [S20] PostgreSQL now PANICs on fsync failure (back-patched to 9.4). Linux 4.13 improved error reporting with `errseq_t`. Darwin invalidates the buffers. **[V]** [S19]
- **Directory fsync.** "Calling fsync() does not necessarily ensure that the entry in the directory containing the file has also reached disk. For that an explicit fsync() on a file descriptor for the directory is also needed." **[V]** [S23]
- **fdatasync.** It skips metadata "unless that metadata is needed" for later reads; file size counts as needed. **[V]** [S23]
- **rename.** An existing target is "atomically replaced" from the point of view of other processes. That says nothing about crash durability, which ALICE shows varies. **[V]** [S23][S17]
- **Torn writes.** Linux 6.11 added `RWF_ATOMIC` for untorn writes, with power-of-2, naturally aligned sizes inside device limits. **[V]** [S23] It's Linux-only and depends on hardware, so we can't rely on it. **[Inf]** Kafka documents both truncated tails and garbage added at the end. **[V]** [S9]
- **macOS.** `fsync` "will flush all data from the host to the drive" but the drive may reorder or lose it. `F_FULLFSYNC` asks the drive to flush. **[V]** [S22] Apple suggests `F_BARRIERFSYNC` for write barriers and `F_FULLFSYNC` "when your app requires a strong expectation of data persistence". **[V]** [S22]
  - **What Rust std does:** on Apple targets both `File::sync_all` and `File::sync_data` call `fcntl(fd, F_FULLFSYNC)`; on Linux they call `fsync` and `fdatasync`. **[V]** `rust-lang/rust:library/std/src/sys/fs/unix.rs:1268-1323` [S21]
- **Preallocation.** PostgreSQL's `wal_init_zero` (on by default) zero-fills new WAL files so "space is allocated before we need to write WAL records", noting copy-on-write file systems "may not benefit". `wal_recycle` renames old files for reuse. `fdatasync` is the default WAL sync method on Linux. **[V]** [S26] LevelDB reserves record type 0 for preallocated regions and skips zero-length type-0 records. **[V]** [S24]
- **Checksums.**
  - LevelDB and RocksDB frame records with CRC32C, and LevelDB masks stored CRCs because "it is problematic to compute the CRC of a string that contains embedded CRCs". **[V]** [S24]
  - RocksDB's recyclable record format adds a log number to "distinguish between records written by the most recent log writer vs a previous one". **[V]** [S25]
  - Kafka uses CRC-32C. **[V]** [S10]
  - TigerBeetle stores block checksums outside the block to catch misdirected writes. **[V]** [S14]
- **Buffered vs `O_DIRECT`.** `O_DIRECT` "does not give the guarantees of the O_SYNC flag", so fsync is still needed. **[V]** [S23] macOS has no `O_DIRECT`, only `F_NOCACHE`. **[V]** [S22] TigerBeetle uses `O_DIRECT` and its own page cache. **[V]** [S14]
- **io_uring.**
  - tokio's `fs` uses `spawn_blocking` and "also uses io_uring for file operations on Linux when compiled with `tokio_unstable`". **[V]** [S29]
  - tokio-uring's last release was 0.5.0 (2024-05-27) and last commit 2025-07-07; glommio's last release was 0.9.0 (2024-03), last main commit 2025-04; monoio's last release was 0.2.4 (2024-08). **[V]** [S30]
  - Google: 60% of kernel exploit submissions to its program used io_uring; it is disabled on ChromeOS and unreachable to Android apps. **[V]** [S27]
  - Docker's default seccomp profile blocks `io_uring_*` (moby commit `891241e7e7`, 2023-11-02). **[V]** [S28]

**Implications for us:**

| Question | Decision | Basis |
|---|---|---|
| fsync vs fdatasync | Linux: `sync_data()` (fdatasync) for log appends; `sync_all()` for directories and manifests. macOS: either calls `F_FULLFSYNC`, which is correct for acknowledgments. | [S21][S23] **[V]** |
| `F_BARRIERFSYNC` | Not for acknowledgment paths. It could order a blob write before its rename, but we fsync both anyway. | [S22] **[Inf]** |
| Preallocation | v0: none. Append plus fdatasync is simple, and APFS is copy-on-write. v1: benchmark zero-filled preallocated segments on ext4/XFS only. | [S26] **[Inf]** |
| Framing checksum | CRC32C (hardware accelerated, standard). Not xxh3, which has no burst-error guarantees and no precedent in storage formats. BLAKE3 for blob identity, segment seals and hash chains. | [S10][S24][S91] **[V]/[Inf]** |
| Group commit | Natural batching: while one fsync runs, queue the next batch; cap by bytes (1–4 MiB) and count; no sleeps. Acknowledge after fsync **and** after the batch is applied to the published snapshot. | **[Inf]** |
| Buffered vs `O_DIRECT` | Buffered for v0 and v1. `O_DIRECT` is Linux-only, needs alignment, and makes us own caching. | [S14][S23] |
| io_uring | Not worth it: not on macOS, write rates are modest and dominated by fsync latency, the security posture is restrictive, and the runtimes are stale. Revisit only for heavy random reads during rescoring on Linux servers. | [S27]–[S30] **[V]/[Inf]** |
| fsync or write error | Stop accepting writes, exit with an error, recover from disk on restart. Never retry fsync. | [S18][S19] |
| `RWF_ATOMIC` | Don't depend on it. Our CRC-plus-truncation scheme already handles torn tails. | [S23] |

### 2.3 Testing storage correctness

**Tools (verified):**

| Tool | What it does | Status | Use for v0? |
|---|---|---|---|
| FoundationDB simulation | Deterministic simulation built into its C++ runtime (Flow); "tens of thousands of simulations every night" **[V]** [S34] | Reference design | Copy the approach |
| TigerBeetle VOPR | Deterministic from a seed and commit; clock, network and disk stubbed; runs time faster than real **[V]** [S35] | Reference design | Copy the approach |
| Antithesis | Commercial, fully deterministic environment **[C]** [S36] | Commercial | Later, for multi-node |
| madsim 0.2.34 | Tokio-like deterministic runtime; I/O mocked **[V]** [S37] | Last release 2025-10 | v2 (network) |
| turmoil 0.7.2 / turmoil-fs 0.1.0 | Deterministic simulation; turmoil-fs models pending vs durable writes, durability of directory entries ("durable when parent dir synced"), torn writes at a configurable block size, and an `io_error_probability` setting that randomly fails reads and writes with EIO **[V]** [S38] (`tokio-rs/turmoil:crates/turmoil-fs/src/lib.rs:365-446,870-883`) | turmoil-fs published 2026-10-19 (27 downloads) — very new | **Evaluate now**; keep our own small fallback |
| shuttle 0.9.10 | Randomized concurrency testing (PCT scheduler) **[V]** [S39] | Active (2026-11-05) | Yes, for reader/writer code |
| loom 0.7.3 | Exhaustive interleavings under the C11 memory model **[V]** [S40] | Active | Only if we write lock-free code |
| stateright 0.31.0 | Actor model checker **[V]** [S41] | Last release 2025-07 | v2 (leases, replication) |
| LazyFS | FUSE file system whose cache "only flushes data if explicitly requested", simulating power loss; VLDB 2024 paper **[V]** [S32] | Linux only, C++ | Linux CI |
| dm-log-writes | Device-mapper target logging every write for replay; used to check file-system integrity **[V]** [S33] | Linux kernel | Optional, deeper |
| CrashMonkey / ACE (B3) | Most reported crash bugs reproduce with ≤3 file operations; all came after fsync-related calls; found 10 new bugs including "broken rename atomicity" **[V]** [S31] | Research | Shape of our workload generator |
| proptest 1.12 + proptest-state-machine 0.9 | Property and state-machine tests **[V]** [S43] | Active | Yes |
| cargo-fuzz 0.13.2 | libFuzzer; needs nightly **[V]** [S42] | Active | Yes (nightly CI job) |

**Recommended v0 test strategy [Inf]:**
1. **Put file operations behind one trait** (open, pwrite, append, read, fsync, fdatasync, rename, unlink, mkdir, fsync of a directory, truncate, list). Production uses `std::fs`. The test implementation is a seeded simulated file system modeling page cache vs durable state, directory-entry durability, torn writes at 512-byte or 4 KiB granularity, EIO on fsync, ENOSPC and short writes. Try turmoil-fs; if its semantics or maturity fall short, our own version is about 500–800 lines.
2. **Crash at every step.** Record the file-operation trace of a workload, then for every prefix and every allowed reordering of unsynced operations, build the post-crash state, run recovery and check the invariants:
   - Every acknowledged event is present, byte for byte.
   - Nothing unacknowledged is half-applied.
   - Sequence numbers are contiguous.
   - Projections equal a replay of the log.
   - Every blob referenced by a durable event exists and matches its hash.
3. **Model-based property tests** of the store API (append, append_batch, read, read-as-of, erase, crash/restart) against a pure in-memory model.
4. **Golden fixtures:** frozen byte files for each frame, envelope and schema version, decoded and upcast in CI forever.
5. **Fuzzing:** frame decoder, recovery scanner on arbitrary segment bytes, envelope and postcard decoding, tokenizer, query parser.
6. **Real-world crashes:**
   - kill -9 loops against the daemon. These test only process crashes: the page cache survives, so they cannot validate fsync placement.
   - LazyFS runs on Linux CI that drop unsynced data.
   - A periodic manual power-cut test on macOS hardware. **[?]** No LazyFS equivalent on macOS.
7. **Concurrency:** shuttle tests on snapshot publication and the writer pipeline; loom only for any custom atomics.
8. **Differential tests:** BM25 with MaxScore vs exhaustive scoring; quantized or approximate vector search vs exact full-precision kNN (recall gates).

### 2.4 Inverted index design

**Postings compression (verified sources):**
- SIMD-BP128 (block bit-packing with SIMD) is "nearly twice as fast" as varint-G8IU and PFOR. SIMD-FastPFOR compresses within 10% of the best scheme. **[V]** [S44] The Rust `bitpacking` crate ports `simdcomp` with 128-integer blocks and a scalar fallback chosen at runtime, and claims ">4 billion integers per second". **[V]/[C]** [S100]
- Stream VByte decodes up to 2× faster than varint-G8IU. **[V]** [S45] The Rust crate (0.4.1, 2023, non-standard license field) is weak. **[V]** [S89]
- Elias-Fano ("quasi-succinct indices") gives expected constant-time operations. **[V]** [S46] Rust options: `sucds` 0.11 and `vers-vecs` 1.10. **[V]** [S89]
- Roaring compresses about 2× better than WAH and Concise, with up to 900× faster intersections. **[V]** [S47][S48] `roaring` 0.12.0 is pure Rust (MSRV 1.90) and serializes "the standard Roaring on-disk format … compatible with the official C/C++, Java and Go implementations". **[V]** [S93]
- PForDelta: Zukowski et al., ICDE 2006. **[V]** [S49]
- Lucene's current postings format (`lucene104`) uses PFOR/FOR blocks of **256** documents, two levels of skip data and **impacts** (per-block score bounds) stored in the skip data. **[V]** [S12] Tantivy uses 128-integer `BitPacker4x` blocks. **[V]** [S13]

**Top-k query processing:**
- WAND (CIKM 2003), Block-Max WAND ("stores the maximum impact score for each block … enabling us to skip large parts"; SIGIR 2011), variable-block BMW (SIGIR 2017) and MaxScore (Turtle & Flood 1995). **[V]** [S50]–[S53]
- Tantivy implements block-WAND (`src/query/boolean_query/block_wand_*.rs`). **[V]** [S13] Lucene runs (filtered) disjunctions through `MaxScoreBulkScorer` (CHANGES GITHUB#14014). **[V]** [S12]
- **When it matters [Inf]:** cost of exhaustive scoring is the sum of document frequencies over query terms. At 10k–1M documents, short queries touch well under 1M postings, about 1–3 ms at roughly 2–5 ns per posting. Agents often paste long error text (30–100 terms), which can reach millions of postings. That's when MaxScore pays off. It also handles long queries well and needs only per-term maximum scores, whereas BMW needs block maxima. Build MaxScore in v1; store block maxima in the v1 segment format so BMW stays possible.

**BM25 variants and parameters:**
- Lucene's `BM25Similarity` defaults to k1 = 1.2, b = 0.75 **[V]** [S12]; Tantivy's are the same constants **[V]** (`quickwit-oss/tantivy:src/query/bm25.rs:8-9`) [S13].
- BM25F weights term frequency *before* the saturation function, equivalent to repeating a field's text. **[V]** [S55]
- BM25+ adds a lower bound to length-normalized term frequency so very long documents aren't over-penalized. **[V]** [S54] This is relevant because our documents range from 200-byte claims to 10 KB episodes. **[Inf]**
- "Which BM25 do you mean?" found "no significant effectiveness differences" among eight variants on newswire collections. **[V]** [S56] So pick BM25F with sane defaults and tune on our own evaluation set rather than chasing variants. **[Inf]**

**Code-aware tokenization [V semantics / Inf design]:**
- Elasticsearch's `word_delimiter_graph` options are a good specification: `split_on_case_change` (camelCase → camel, Case), `split_on_numerics` (j2se → j, 2, se), `preserve_original`, `catenate_all`. **[V]** [S59]
- GitHub's code search uses an n-gram (trigram) index for substrings **[V]** [S57], following Russ Cox's trigram index. **[V]** [S58]
- Our v0 pipeline:
  1. Unicode NFKC and lowercase in a copy (keep the original for display).
  2. Split on whitespace and most punctuation, but emit the **original compound** for identifier-like tokens (`foo_bar`, `FooBar::baz`, `a.b.c`, `--flag`).
  3. Sub-tokens from camelCase (acronym-aware: `HTTPServerError` → http, server, error), snake_case, kebab-case, and letter/digit boundaries.
  4. Versions: keep `v1.2.3` as `1.2.3` plus prefixes `1.2` and `1`. Don't depend on the `semver` crate; ecosystems differ (PEP 440, etc.). **[Inf]**
  5. Paths: the whole path, base name, each component and the extension.
  6. Error codes (`E0502`, `ENOENT`, `0x80070005`): one token each.
  7. Hashes, UUIDs and hex longer than 12 characters go only into an exact-match keyword field. This bounds the vocabulary.
  8. No stemming in code fields; optional light English stemming in prose fields.
- Phrase queries in v0: no positions; check phrases by scanning stored text of the top ~100 candidates. Positions or trigrams come in v1 or v2 for a dedicated code/error field.

**Incremental indexing with immutable segments and deletes:**
- **Lucene/Tantivy model [V]** [S12][S13]:
  - An in-memory buffer is flushed to an immutable segment.
  - Deletes go into per-segment alive bitsets. In Tantivy the bitset files are themselves immutable, named `segment_id.commit_opstamp.del`.
  - Merges run on a tiered policy, and a manifest commit makes changes visible.
- **For us [Inf]:**
  - Index segments are derived data. The manifest records "indexed through sequence S", and recovery replays the log after S.
  - Supersession or retraction marks the old item's document ID dead in the alive bitmap.
  - BM25 statistics (document frequency, total and average lengths) are summed per segment at query time. Deleted documents stay in document frequency until a merge, as in Lucene.

**Footprint estimates [Inf; assumptions stated]:** assume about 100 distinct indexed terms per item on average, with d = 768 for vectors.

| Items | Postings | Uncompressed (u32 doc + u16 tf, about 6 B) | Compressed blocks (about 1.5–2 B) | Vocabulary (code text; **[?]**, measure) |
|---|---|---|---|---|
| 10k | 1M | 6 MB | 2 MB | ~0.1–0.5M terms |
| 100k | 10M | 60 MB | 15–20 MB | ~1–3M terms |
| 1M | 100M | 600 MB | 150–200 MB | ~5–20M terms → a hash-map dictionary at about 50–70 B per term is 0.25–1.4 GB |

**Build first vs later:**
- **v0:** tokenizer with tests and fuzzing; in-memory postings (`Vec<u32>` document IDs and `u16` term frequencies per term); BM25F scoring every match into a dense score array, then a top-k heap; roaring filter intersection; stored text for phrase checks.
- **v1:**
  - Immutable segments: front-coded or `fst` term dictionary, 128-document bit-packed blocks via the `bitpacking` crate, per-block maximum scores, alive bitsets, tiered merges.
  - MaxScore.
  - Token caps to keep the vocabulary bounded.
- **v2:** positions; a trigram field for substring and regex search over artifacts on servers; BMW if profiling shows a need.

### 2.5 Vector index design

**Evidence:**
- **Flat scan plus quantization:**
  - Binary quantization gives a 32× size reduction. On MTEB Retrieval with mxbai-embed-large-v1 it retains 92.53% of quality, rising to 96.45% after rescoring with the full-precision query. **[C/I]** (Hugging Face blog) [S76]
  - Int8 is a 4× reduction. **[V]** [S76]
  - Engines prefer exact scans below small thresholds. Qdrant's `full_scan_threshold` is 10,000 KB (about 10k vectors of 256 dimensions). **[V]** [S77] Weaviate's "dynamic" index starts flat and switches to HNSW at 10,000 objects by default, aimed at multi-tenancy. **[V]** [S78]
  - Laptop scan measurements are in research/02 §4 (for example sqlite-vec: 1M×128 full precision in 17 ms on an M1). [S103]
- **RaBitQ (SIGMOD 2024).** Quantizes D dimensions into D bits with a "sharp theoretical error bound"; notes that PQ "fail[s] disastrously on some real-world datasets". **[V]** [S60] Extended RaBitQ (PACMMOD 3(3), June 2025) adds multi-bit codes and is asymptotically optimal. **[V]** [S61][S62] The reference library (C++/Python) has IVF-RaBitQ, HNSW-RaBitQ and SymphonyQG. **[V]** [S62] Rust RaBitQ crates are tiny (`rabitq` 0.2.2 from 2024, `rabitq-rs` 0.9.0). **[V]** [S89]
- **HNSW.** [S63] Real-time deletes and updates degrade performance and create "unreachable points". **[V]** [S64]
- **DiskANN family.**
  - DiskANN (NeurIPS 2019) [S65].
  - FreshDiskANN: the first graph index to reflect corpus updates without periodic rebuilds. **[V]** [S66]
  - IP-DiskANN (2025): processes each insert and delete in place, without FreshDiskANN's batch consolidation. **[V]** [S67]
- **SPANN / SPFresh.** SPANN keeps centroids in memory and posting lists on disk (NeurIPS 2021). **[V]** [S68] SPFresh does incremental in-place updates at billion scale (SOSP 2023). **[V]** [S69] Weaviate now ships an "HFresh" index that "avoids keeping the full index in memory". **[V]** [S78]
- **IVF-PQ.** Jégou et al., TPAMI. **[V]** [S70]
- **Filtered ANN.** ACORN does "predicate subgraph traversal" on HNSW and is predicate-agnostic (SIGMOD 2024). **[V]** [S71] Filtered-DiskANN builds graphs aware of labels and is an order of magnitude better for filtered queries (WWW 2023). **[V]** [S72]
- **microsoft/DiskANN is now "DiskANN3", a composable Rust library** (MIT, crates `diskann*` 0.61.0, published 2026-10-26).
  - "Implement the `DataProvider` trait for your store". **[V]** [S73]
  - Real-time updates use IP-DiskANN and FreshDiskANN logic; there are hooks for attribute filters. **[V]** [S73]
  - The trait has associated `Context`, `InternalId`, `ExternalId`, `Error` and `Guard` types, plus sub-traits `Delete` and `SetElement<T>`. **[V]** `microsoft/DiskANN:diskann/src/provider.rs:98-200`
  - `diskann-quantization` offers Scalar, Binary, Product, a RaBitQ-based "Spherical" quantizer, and a training-free per-vector "MinMax" quantizer. **[V]** `microsoft/DiskANN:diskann-quantization/src/lib.rs:11-32`
  - The dependencies are pure Rust (`faer` for linear algebra); the workspace warns on undocumented `unsafe`. **[V]** [S73]
- **SIMD in Rust:**
  - `std::simd` is still unstable (tracking issue #86656 open; `#[unstable(feature = "portable_simd")]`). **[V]** [S74]
  - `pulp` 0.22.3 ("safe generic simd"), `wide` 1.7.1, `fearless_simd` 1.2.0 (used by roaring-rs), `diskann-wide` and `diskann-vector` 0.61. **[V]** [S89][S93]
  - The SimSIMD GitHub repo now redirects to **NumKong**. The `numkong` crate was created 2026-03-17; `simsimd`'s last release was 6.5.16 on 2026-03-07. It's a C library built with CMake (`[build-dependencies] cmake`), and main's Cargo.toml declares `rust-version = "1.100"`. **[V]** [S75]

**Exact vs approximate below ~1M [Inf, using research/02 and bandwidth arithmetic]:**

| d = 768, per scope | f32 | int8 (+8 B per vector) | 1-bit |
|---|---|---|---|
| 100k vectors | 307 MB | 77 MB | 9.6 MB |
| 1M vectors | 3.07 GB | 776 MB | 96 MB |

At an assumed 20–40 GB/s effective memory bandwidth on one core, a 1M 1-bit scan takes about 3–5 ms. A 1M int8 scan takes about 20–40 ms single-threaded (5–10 ms on four threads). Rescoring 100–200 candidates in full precision is negligible from RAM and costs 100–200 random reads from SSD.

An exact scan under a roaring filter has **perfect recall at any selectivity**, at a cost proportional to the number of allowed IDs. Filtered approximate search needs special algorithms (ACORN, Filtered-DiskANN) to avoid recall collapse. **[V]** [S71][S72]

**Build first vs later [Inf]:**
- **v0:** Per store, keep full-precision vectors in an append-only file by internal ID and int8 MinMax codes (per-vector scale and offset, streaming-friendly like DiskANN's MinMax). Exact scan over IDs allowed by the filter, rescoring the top 4×k in full precision.
  - Write our own kernels for dot product (f32 and i8→i32), L2 and Hamming distance (popcount over u64): start with loops the compiler vectorizes, then add `std::arch` paths chosen at runtime where profiling says so.
  - Property-test every kernel against a scalar reference.
- **v1:** a 1-bit first stage (sign bits, or RaBitQ with random rotation and correction factors), then int8, then full precision, for 0.2–2M vectors. Vector code files become immutable segments alongside text segments.
- **v2:** an approximate index only when exact search misses the latency budget at the 95th percentile on real filtered data. Candidates, evaluated against the exact baseline for recall:
  - **IVF+RaBitQ:** cluster lists fit immutable segments and merges, and filters intersect easily.
  - **DiskANN3 through `DataProvider`:** in-place updates and filter hooks, but young versions at 0.61.
  - **HNSW:** last, given its delete pathologies. **[V]** [S64]

### 2.6 Graph adjacency, temporal metadata and filters

- **Adjacency [Inf]:**
  - Edges are events: (edge ID, source, destination, type, valid-time interval, sequence number).
  - The snapshot form is a compressed sparse row (CSR) layout for out- and in-edges over dense node IDs, plus a small structure of recent additions.
  - Traversals of 1–3 hops are breadth-first with a visited bitmap, per-hop fan-out caps, and edge filtering by alive, scope and status bitmaps.
  - Estimate: 5M edges × 2 directions × 12 B ≈ 120 MB.
- **Bitmap filters [V/Inf]:** one roaring bitmap per scope, kind and status value (and per applicability condition later) over per-store document IDs. Compose by AND/OR/ANDNOT, then feed the BM25 scorer, the vector scan and the graph walk.
- **Temporal [Inf]:**
  - System time is the log sequence number. "As of" queries cover only the log and metadata, so the text and vector indexes need no history.
  - Each claim, entity or lease keeps a version chain `[(seq_from, seq_to, state)]`. As-of(S) uses a binary search over that chain. Deep audits replay the log from the nearest checkpoint.
  - Valid time (applicability) is stored as interval fields: scan in v0, sorted endpoints or an interval tree in v1.
  - Main-memory temporal indexes for later: Timeline Index (SIGMOD 2013) and HINT (SIGMOD 2022). **[V]** [S82][S83]
- **Persistent and concurrent structures [V]** [S88][S89][S101]:
  - `im` is unmaintained (RUSTSEC-2026-0248; recommends `imbl`). `imbl` 7.0.2 is MPL-2.0 (file-level copyleft), MSRV 1.85.
  - `rpds` 1.2.1 is MIT.
  - `arc-swap` 1.9.5 is an `RwLock<Arc<T>>`-like way to swap in a new snapshot pointer.
  - `left-right` 0.11.8 keeps two copies of the data, doubling memory; `evmap` 11.0.0 is built on it.
  - `crossbeam-epoch` 0.9.21 (RUSTSEC-2026-0204 fixed in 0.9.20 or later).
- **Recommendation [Inf]:** `ArcSwap<Snapshot>`, where `Snapshot` holds `Arc`s to immutable segments plus a frozen copy of the per-batch delta. Use `imbl` or `rpds` maps only for small, frequently changing projections (leases, claim status). Avoid `left-right` (2× memory) and avoid epoch-based lock-free code unless profiling demands it.

### 2.7 Concurrency architecture in Rust

- **Single writer plus snapshot readers [Inf]:**
  - A dedicated `std::thread` owns the active segment, the sequence allocator and projection updates.
  - Request handlers do expensive work first: decode, validate, hash and persist blobs, tokenize, compute vector codes. They then send ready-to-append frames over a bounded channel to the writer.
  - The writer loop: collect a batch, `write` (or `writev`), `sync_data`, apply, build the delta, `ArcSwap::store` the new snapshot, then acknowledge through oneshot channels.
  - Background threads build and merge segments and send results back to the writer, which alone mutates the manifest.
- **tokio vs threads:**
  - Tokio's file operations run on the `spawn_blocking` pool, and cancelling a write future doesn't stop the write. **[V]** [S29] tokio recommends dedicated executors such as rayon for CPU-bound work. **[V]** [S29]
  - So use tokio for sockets, gRPC and MCP; a dedicated writer thread; and a rayon or fixed thread pool for search.
  - Use std `Mutex` by default: it's futex-based on Linux since Rust 1.62. **[V]** [S98] `parking_lot` 0.13 needs MSRV 1.95. **[V]** [S89]
- **mmap pitfalls:**
  - Crotty, Leis and Pavlo (CIDR 2022): no transactional safety (the OS may flush dirty pages at any time), I/O stalls on page faults, awkward error handling (SIGBUS), and TLB shootdowns. Advice: don't use mmap for updates or when you care about error handling; "maybe" only when the working set fits in memory and the workload is read-only. **[V]** [S79]
  - memmap2 marks all file-backed constructors `unsafe` because of "Undefined Behavior … if the underlying file is subsequently modified". **[V]** (`RazrFalcon/memmap2-rs:src/lib.rs:230-235`) [S90] RUSTSEC-2026-0186 is fixed in 0.9.11 or later. **[V]** [S88]
  - Decision: `pread` for the log and blobs. mmap is allowed in v1 only for sealed, immutable, read-only index files we own, with a `SIGBUS` policy and a pread fallback.
- **Buffer manager:** LeanStore (ICDE 2018) and vmcache (SIGMOD 2023) show high-performance buffer management **[V]** [S80][S81]. It's not warranted for a single-node laptop store whose files are immutable and served by the page cache. Revisit only for v2 servers with data much larger than RAM and an `O_DIRECT` path. **[Inf]**

### 2.8 Serialization and format evolution

| Option | Facts (verified) | Verdict |
|---|---|---|
| serde_json 1.0.154 | Ubiquitous **[V]** [S89] | API, MCP, debug dumps; not storage |
| postcard 1.1.3 | Wire format "considered stable as of v1.0.0"; "does NOT enforce canonicalization" when decoding **[V]** [S85]; last release 2025-07, repo active 2026-11 **[V]** [S89] | **Storage payloads**, inside a versioned envelope |
| bincode 3.0.0 | Unmaintained after "a doxxing and harassment incident"; 1.3.3 declared complete (RUSTSEC-2025-0141) **[V]** [S88] | Avoid |
| rkyv 0.8.18 | Own comparison table says schema evolution: "no" **[V]** [S86]; advisories in 2026: 0001, 0122, 0233 (use-after-free from crafted archives), 0234, 0235 (all fixed by 0.8.17) **[V]** [S88] | Avoid for persisted formats |
| CBOR (RFC 8949 §4.2.1) | Core deterministic rules: preferred (shortest) serialization, no indefinite lengths, map keys sorted bytewise by their encodings **[V]** [S84]. `ciborium` 0.2.2 (2024-01) has only `cmp_value`; its canonical writer exists on main but is unreleased, sorts length-first (RFC 7049 style), and recent commits are dependency bumps **[V]** [S87]. `minicbor` 2.3.1 (BlueOak-1.0.0) is active **[V]** [S89] | Use only if cross-language canonical bytes are needed (COSE signatures, v2) |
| FlatBuffers 25.12.19 / capnp 0.27.3 | Schema evolution plus zero-copy **[V]** [S86]; past advisories (capnp RUSTSEC-2025-0143, fixed in 0.24 or later) **[V]** [S88] | Not needed; adds code generation |

**Decisions [Inf]:**
1. **Envelope:** `{envelope_version, event_type: u16, schema_version: u16, flags, payload: bytes}`. The payload is postcard of `EventVn`. Readers keep a decoder for every historical version and chain upcasters (v1→v2→…) at read time. The log is never rewritten for schema changes. Projection, checkpoint and index formats carry their own version and are simply rebuilt from the log when they change.
2. **Hash what you store:**
   - Frame integrity: CRC32C over the exact bytes.
   - Artifact identity: BLAKE3 of the raw artifact bytes.
   - Signatures (future): over the exact payload bytes the client sent, which we store verbatim.
   - Never re-encode before verifying; postcard isn't canonical.
3. **Semantic de-duplication:** a separate normalized key, for example BLAKE3 over normalized claim text, scope and kind. Don't rely on canonical encoding for this.
4. **Hash chain:** chain *record digests* (header plus BLAKE3 of the payload), not payload bytes. Erasing a payload then leaves the chain verifiable. Seal segments with BLAKE3 and record the seal in the manifest, i.e. the checksum lives in the parent, as TigerBeetle does [S14].

### 2.9 Proven crates for building blocks (data from crates.io, GitHub and RustSec, checked 2026-11-10)

| Crate (version, last release) | License / MSRV | Maturity signals | Notes and soundness | Verdict |
|---|---|---|---|---|
| **crc32c** 0.6.9 (2026-11-02) | MIT/Apache | 99M downloads | CRC-32C with SSE4.2 and aarch64 acceleration, software fallback **[V]** [S92] | **Use** for framing |
| crc32fast 1.5.5 (2026-11-10) | MIT/Apache, 1.63 | 855M downloads | **CRC32 (IEEE), not CRC-32C** **[V]** [S92] | Don't confuse the two |
| crc-fast 1.10.0 (2025-12-31) | MIT/Apache, 1.89 | 78M downloads | Generic SIMD CRC; claims >100 GiB/s **[C]** [S92] | Alternative |
| **blake3** 1.8.7 (2026-08-20) | CC0/Apache | 224M downloads | x86_64 uses **assembly via `cc` by default**; the `pure` feature forces Rust only **[V]** (`BLAKE3-team/BLAKE3:Cargo.toml:80-91,138-139`) [S91] | **Use** (`pure` if all-Rust is strict) |
| xxhash-rust 0.8.19 / twox-hash 2.1.5 | BSL-1.0 / MIT | 139M / 296M downloads | Non-cryptographic | Optional (fingerprints) |
| **roaring** 0.12.0 (2026-10-28) | MIT/Apache, **1.90** | 63M downloads | Pure Rust; standard portable format **[V]** [S93] | **Use** |
| zstd 0.14.3 (2026-11-01) | BSD-3, 1.64 | 473M downloads | **C libzstd binding**; three advisories on 2026-10-16, fixed in 0.14.3 or later **[V]** [S88] | Artifacts only, if C is acceptable |
| ruzstd 0.9.1 (2026-10-11) | MIT, 1.87 | 79M downloads | Pure Rust; complete decoder (author measured about 3.5× slower than C); encoder "usable" but not at C speed or ratio **[C]** [S94] | Pure-Rust option for decoding |
| **lz4_flex** 0.14.0 (2026-07-14) | MIT, 1.81 | 171M downloads | Safe by default (`safe-encode`/`safe-decode`); RUSTSEC-2026-0041 fixed in 0.12.1 or later **[V]** [S94][S88] | **Use** for segment blocks (v1) |
| memmap2 0.9.12 (2026-10-21) | MIT/Apache, 1.65 | 421M downloads | Constructors are `unsafe`; RUSTSEC-2026-0186 fixed in 0.9.11 or later **[V]** | Defer (v1, read-only) |
| **postcard** 1.1.3 (2025-07-24) | MIT/Apache | 91M downloads | Stable wire format; not canonical **[V]** | **Use** |
| **serde_json** 1.0.154 | MIT/Apache, 1.71 | 1.59B downloads | — | **Use** (API) |
| rkyv 0.8.18 / bincode 3.0.0 | MIT | — | See §2.8 | Avoid |
| parking_lot 0.13.0 (2026-10-21) | MIT/Apache, **1.95** | 1.26B downloads | — | Optional; prefer std |
| crossbeam 0.8.5 / -channel / -epoch 0.9.21 | MIT/Apache | 159M / 805M downloads | RUSTSEC-2025-0024 (channel ≥0.5.15), 2026-0204 (epoch ≥0.9.20) **[V]** | Channel OK; avoid epoch code |
| **arc-swap** 1.9.5 (2026-11-09) | MIT/Apache | 403M downloads | Old RUSTSEC-2020-0091 only | **Use** |
| **tokio** 1.54.1 (2026-10-21) | MIT, 1.85 | 1.18B downloads | **Pin ≥1.54.1** (RUSTSEC-2026-0384 race in 1.54.0); 2026-0386 fixed in 1.53.3 or later **[V]** | **Use** (network layer) |
| simsimd 6.5.16 → numkong 7.8.7 | Apache-2.0 | 4.3M / 0.4M downloads | C library, CMake build **[V]** [S75] | Avoid (all-Rust goal) |
| **half** 2.7.1 / **bytemuck** 1.25.2 | MIT/Apache; Zlib/Apache/MIT | 562M / 468M downloads | — | **Use** |
| std `File::lock`/`try_lock` | Rust ≥1.89 | stable `#[stable(since="1.89.0")]` **[V]** [S21] | `flock`-based on Unix, advisory **[V]** (`library/std/src/sys/fs/unix.rs:1325-1347`) | **Use** (no fs2/fs4 needed) |
| fs4 1.1.0 (2026-04-28) / fs2 0.4.3 (2018) | MIT/Apache | 80M / 102M downloads | fs4 `allocate` uses rustix `fallocate` (and `F_PREALLOCATE` on Apple) **[V]** [S96] | Optional; or use `rustix` directly |
| **uuid** 1.29.2 (2026-11-09) | Apache/MIT, 1.89 | 976M downloads | `v7` feature; `now_v7` ordered within a process **[V]** [S97] | **Use** |
| semver 1.0.28 | MIT/Apache, 1.68 | 1.19B downloads | Cargo SemVer only | Optional (not for tokenizing) |
| **proptest** 1.12.0 + proptest-state-machine 0.9.0 | MIT/Apache, 1.88 | 241M downloads | — | **Use** |
| **tempfile** 3.27.0 | MIT/Apache, 1.63 | 982M downloads | `persist` syncs **neither file nor directory** **[V]** [S95] | Tests only; never the blob store |
| bitpacking 0.9.3 / fst 0.4.7 (2021) | MIT / Unlicense-MIT | 30M / 34M downloads | SIMD-BP128 port **[V]**; FST is stable and done | v1 segments |
| diskann / -quantization / -vector 0.61.0 | MIT | 657k downloads | Pure Rust; young version numbers **[V]** [S73] | v2 candidate / reference |

Rust stable is 1.99.1 (2026-10-16). **[V]** [S99] Suggested minimum Rust version: about 1.95–1.97 to accommodate roaring and parking_lot. **[Inf]**

---

## 3. Recommended engine architecture

### v0 (implement now)

1. **Store:** the directory layout in §2.1. On startup, `LOCK.try_lock()`; refuse to start if another process holds it.
2. **Frame** (little-endian, 8-byte aligned):

   ```
   u32 body_len            (1..=MAX_FRAME e.g. 16 MiB; 0 is invalid)
   u32 crc                 masked CRC32C over [body_len ‖ bytes 8..end of body]
   u8  frame_type          (0 reserved = invalid; 1 = EventBatch; 2 = SegmentFooter; …)
   u8  flags               (compressed, redacted, …)
   u16 header_version
   u32 segment_epoch       (low 32 bits of segment id; catches stale or misdirected frames)
   u64 first_seq           (events in the batch take first_seq..first_seq+n)
   body: envelope(s)       (postcard), then zero padding to 8 bytes
   ```

   One client request is one frame, so multi-event requests are atomic.
3. **Write path:**
   1. The handler validates the request and writes any blobs, which must be durable before the event is accepted.
   2. The handler precomputes tokens and vector codes, then sends the frame to the writer.
   3. The writer groups frames, appends, calls `sync_data`, applies them, publishes the snapshot and acknowledges.
   4. Segments roll at 64–256 MiB. To roll: write a footer, `sync_data`, create the next file, `sync_all` the log directory, then update the manifest (A/B slots, generation counter, checksum).
4. **Recovery:**
   1. Load the newest valid manifest slot.
   2. Check that the sealed segments listed exist; verify their BLAKE3 seals lazily or in a scrub.
   3. Scan the active segment's frames, checking length bounds, CRC, epoch and sequence continuity.
   4. At the first bad frame, probe forward for any valid later frame. If one exists, stop with a corruption error. If not, cut off the file there and fsync it.
   5. Replay to build projections and indexes, and empty `blobs/tmp`.
5. **Projections:** claims and their status history (version chains), scopes, entities, edges (compressed sparse row plus delta), leases (fencing token = sequence number of the grant), idempotency keys (hash map, with a time-to-live by sequence).
6. **Indexes:** in-memory BM25F with the code tokenizer; roaring filters; full-precision vectors in an append-only file plus int8 codes in memory; exact scan with rescoring.
7. **Reads:** handlers take `snapshot = ArcSwap::load_full()`, compute the filter bitmap, then run BM25 and vector scans in parallel on the search pool. Results are fused with reciprocal rank fusion (RRF) or weighted fusion (designed in docs/), expanded 1–3 hops, and hydrated from the log with `pread` of frames by sequence through the sparse index.
8. **Budgets:** about 200k items or fewer per store in memory on a laptop. Expose RSS, replay time and fsync latency metrics.

### v1

- Projection checkpoints (versioned files written via temp+fsync+rename+directory fsync, recording the sequence they cover) so startup is fast.
- Immutable text segments: `fst` or front-coded dictionary, 128-document blocks with block maxima, alive bitsets, Lucene-like tiered merges (8 per tier, floor ~16 MB).
- MaxScore; 1-bit or RaBitQ vector first stage.
- Segment compression with lz4_flex.
- A scrubber that verifies seals and blob hashes.
- Erasure by rewriting segments: a redaction frame plus compaction that rewrites the affected segments with payloads removed and digests kept.

### v2

- Followers that ship segments (sealed segments plus the active tail streamed with fencing); partitioning by (tenant, project).
- Sorted-run indexes for random keys at 10M+.
- An approximate vector index chosen by evaluation (IVF+RaBitQ vs DiskANN3).
- Optional `O_DIRECT` and our own cache on Linux servers.
- Trigram or position index for code and error fields.
- Optional deterministic CBOR/COSE for signed claims.

### Component table

| Component | Build or use | Why |
|---|---|---|
| Segment log, framing, group commit, recovery, manifest | **Build** | Core; requirements are specific (torn-tail vs corruption policy, epochs, seals) |
| Blob store (content-addressed, GC, erasure) | **Build** (blake3) | Simple; durability details matter (tempfile doesn't sync) |
| File abstraction plus crash simulator | **Build** (evaluate turmoil-fs) | Testability is the backbone |
| Projections, version chains, adjacency | **Build** (std, hashbrown; optionally imbl/rpds) | Domain-specific |
| Tokenizer, BM25F, postings, segments, merges | **Build** (bitpacking, roaring, optionally fst) | Code-aware ranking is our differentiator; Tantivy is excluded |
| Vector store, quantizers, kernels | **Build** (half, bytemuck; diskann-quantization as reference or option) | Small, testable; avoids C SIMD libraries |
| Approximate index (v2) | **Evaluate** DiskANN3 (`DataProvider`) vs our own IVF+RaBitQ | Avoid premature commitment |
| Checksums and hashes | **Use** crc32c, blake3 | Commodity, hardware-accelerated |
| Serialization | **Use** postcard, serde_json; build the envelope and upcasters | Stable wire format; evolution stays under our control |
| Compression | **Use** lz4_flex (segments); zstd or ruzstd (artifacts) **[?]** | Commodity |
| Async and networking | **Use** tokio ≥1.54.1 | Commodity |
| Concurrency primitives | **Use** std sync, arc-swap, crossbeam-channel ≥0.5.15 | Mature |
| Locking, preallocation, stats | **Use** std `File::try_lock`; rustix | std is enough |
| IDs | **Use** uuid (v7) | Standard |

---

## 4. WAL and blob-store correctness checklist, and test strategy

### Write-ahead log and segment log

1. Single writer enforced by `File::try_lock` on `LOCK`, held for the life of the process.
2. Every frame has a non-zero length below a maximum, a non-zero type, a header version, a segment epoch and a sequence number. The masked CRC32C covers the length and all header fields after the CRC, plus the body.
3. Sequence numbers increase strictly and contiguously across segments; check this during recovery.
4. Acknowledge only after the segment's `sync_data()` returns successfully, which is `F_FULLFSYNC` on macOS.
5. New segment: create, write header, `sync_data`, then `sync_all()` the `log/` directory before any frame in it is acknowledged.
6. Seal: write the footer (count, last sequence, BLAKE3 of the content), `sync_data`, then update the manifest.
7. Manifest: A/B slots with generation counter and checksum; write the slot, then `sync_all`; never overwrite the slot currently in use. (Or temp+fsync+rename+directory fsync.)
8. Any `EIO`, `ENOSPC` or short write on write or fsync stops the process; recover from disk on restart; never retry fsync.
9. Recovery cuts off only a torn tail (no valid frame after the bad one) and fsyncs after cutting. A bad frame followed by valid frames is a hard error, with an explicit repair command.
10. Readers never see unacknowledged data: publish the snapshot after fsync.
11. Retried client requests are de-duplicated with idempotency keys stored in the log.
12. Block new writes below a free-disk threshold (`statvfs` via rustix) before ENOSPC hits.
13. Sparse sequence and time indexes are rebuildable, and are checked or rebuilt on any mismatch (Kafka doesn't checksum them; we can rebuild them cheaply).
14. Segment files that are preallocated or zero-filled (if ever used) rely on rule 2: zero type and zero length are invalid.

### Blob store

1. Stream into `blobs/tmp/<random>`, hashing with BLAKE3 as the bytes arrive.
2. `sync_all()` the temp file.
3. Create fan-out directories if missing and `sync_all()` their parent when a new one is created.
4. `rename(tmp, blobs/ab/cd/<hash>)`. If the target exists, delete the temp file; the content is identical by hash.
5. `sync_all()` the target directory. Optionally fsync `tmp/`; leftovers are swept on startup.
6. Only then accept the event that references the blob. The writer checks the blob exists before appending.
7. Reads check BLAKE3 (streaming or verified ranges) at least on first read and during scrubs.
8. GC: mark reachable hashes from projections; sweep unreferenced blobs older than a grace period; fsync the directories.
9. Erasure: append a redaction event, unlink the blob, fsync its directory, and rewrite log segments containing the payload. Document the caveat that SSDs and copy-on-write file systems such as APFS may keep old blocks. **[Inf]**

### Test strategy (v0 gates)

- Simulated file system: about 10⁵ seeded crash schedules per CI run on file-operation traces of 50–500 operations. Assert the invariants in §2.3. Torn writes at 512 B and 4 KiB.
- Model-based tests of the store API, with crash/restart as an operation.
- Fuzzing (nightly): frame decoder and recovery scanner; envelope and postcard decoding; tokenizer.
- Golden fixtures for every format version.
- LazyFS on Linux CI; kill -9 loops on macOS and Linux.
- Shuttle tests for the writer pipeline and snapshot publication.
- Differential tests: BM25 optimizations vs exhaustive scoring; quantized vectors vs exact full precision (recall@10 ≥ target).
- Benchmarks: fsync latency per platform, replay MB/s, peak RSS at 10k/100k/1M items.

---

## 5. Risks and how to retire them

| Risk | Retire by |
|---|---|
| A subtle durability bug (missing directory fsync, wrong ordering) | Simulated file-system crash exploration, LazyFS, ALICE-style workload generator; review against the §4 checklist |
| macOS `F_FULLFSYNC` latency limits write throughput | Group commit; measure on Apple-silicon laptops; keep acknowledgment semantics strict (no "relaxed" default) |
| Memory blow-up at 1M items (vocabulary, postings, vector codes) | Token caps, a budget test at 1M synthetic items, on-disk segments in v1 |
| Startup replay time grows with the log | Checkpoints early in v1; track replay MB/s |
| Format lock-in or evolution mistakes | Envelope plus upcasters, golden fixtures, hashing exact bytes |
| Erasure conflicts with immutability, hash chains, backups and replicas | Chain digests not payloads; compaction by rewriting segments; erasure events propagate to replicas; document SSD remanence |
| Filtered vector recall or latency beyond 1M | Exact scan as default; gate any approximate index on recall and 95th-percentile latency on real filtered data |
| Crate churn and soundness (rkyv, tokio 1.54.0, zstd) | `cargo-deny` and `cargo-audit` in CI; pinned minimums (tokio ≥1.54.1, memmap2 ≥0.9.11, crossbeam-channel ≥0.5.15, zstd ≥0.14.3) |
| Long-lived reader snapshots pin old segments, like LMDB readers [S8] | Query deadlines; a metric for snapshot age; segments freed by reference count |
| Scope creep from "own everything" | Strict v0 component list; reuse crates for commodity work |

---

## 6. Open questions

1. Should IDF and average length be computed per scope or per store? This matters if scopes differ a lot in size or vocabulary.
2. What must erasure purge: log payloads, postings, vectors, checkpoints, replicas, backups? Is crypto-shredding (per-scope keys) acceptable for backups?
3. Do directory fsyncs through `F_FULLFSYNC` behave as expected on APFS, and which errors can they return? **[?]** This needs a test.
4. Embedding dimensions and models per store, and how often stores are re-embedded. This sets vector file layout and the exact-scan thresholds.
5. Are phrase queries or positions required in v0 for error-string recall, or is checking phrases on the top candidates enough?
6. Compression for artifacts: is the C libzstd binding acceptable, or must everything be pure Rust (ruzstd decoding, lz4_flex)?
7. Is "sign exact bytes" enough for future signed claims, or do we need canonical cross-language CBOR (COSE)?
8. Content hashes of low-entropy secrets can be confirmed by guessing. Should blob identity use keyed BLAKE3 per tenant?
9. Does turmoil-fs 0.1.0 model enough (EIO on fsync, directory semantics after `rename`) to replace our own simulator?

---

## 7. Sources (all checked 2026-11-10)

1. Sheehy & Smith, "Bitcask: A Log-Structured Hash Table for Fast Key/Value Data" (Basho, 2010) — https://riak.com/assets/bitcask-intro.pdf
2. Lu et al., "WiscKey: Separating Keys from Values in SSD-conscious Storage", FAST 2016 — https://www.usenix.org/conference/fast16/technical-sessions/presentation/lu
3. Chandramouli et al., "FASTER: A Concurrent Key-Value Store with In-Place Updates", SIGMOD 2018 — https://doi.org/10.1145/3183713.3196898
4. microsoft/FASTER README ("no longer actively maintained"; points to Garnet/Tsavorite) and `docs/_docs/20-fasterkv-basics.md` (HybridLog regions) — https://github.com/microsoft/FASTER
5. O'Neil, Cheng, Gawlick & O'Neil, "The log-structured merge-tree (LSM-tree)", Acta Informatica 33(4), 1996 — https://doi.org/10.1007/s002360050048
6. Dayan, Athanassoulis & Idreos, "Monkey: Optimal Navigable Key-Value Store", SIGMOD 2017 — https://doi.org/10.1145/3035918.3064054
7. Dayan & Idreos, "Dostoevsky: Better Space-Time Trade-Offs for LSM-Tree Based Key-Value Stores via Adaptive Removal of Superfluous Merging", SIGMOD 2018 — https://doi.org/10.1145/3183713.3196927
8. LMDB `lmdb.h` (main page and caveats) and `mdb.c:1260-1261` (`NUM_METAS 2`) — https://git.openldap.org/openldap/openldap/-/blob/mdb.master/libraries/liblmdb/lmdb.h ; https://git.openldap.org/openldap/openldap/-/blob/mdb.master/libraries/liblmdb/mdb.c
9. Apache Kafka, `docs/implementation/log.md` (lines 45, 72, 76–78: segment lookup, copy-on-write segment list, recovery and truncation, "nonsense data") — https://github.com/apache/kafka/blob/trunk/docs/implementation/log.md
10. Apache Kafka, `docs/implementation/message-format.md:40-67` (CRC-32C over the record batch) — https://github.com/apache/kafka/blob/trunk/docs/implementation/message-format.md
11. Apache Kafka, `OffsetIndex.java` and `TimeIndex.java` class Javadoc (sparse, preallocated, memory-mapped, not checksummed, rebuilt after a crash) — https://github.com/apache/kafka/tree/trunk/storage/src/main/java/org/apache/kafka/storage/internals/log
12. Apache Lucene source (main):
    - `BM25Similarity.java:99-110` (k1 = 1.2, b = 0.75)
    - `TieredMergePolicy.java:77-83` (merge defaults)
    - `codecs/lucene104/Lucene104PostingsFormat.java:61-62,116-121,163-171` (256-document blocks, two-level skip data, impacts)
    - `DirectoryReader.java:309-311` (near-real-time readers)
    - `search/MaxScoreBulkScorer.java`
    - `lucene/CHANGES.txt` (GITHUB#14014)

    https://github.com/apache/lucene
13. Tantivy source (main):
    - `ARCHITECTURE.md:41-62` (segments, `meta.json`, alive bitsets)
    - `src/query/bm25.rs:8-9`
    - `src/postings/compression/mod.rs:3`
    - `src/indexer/log_merge_policy.rs:8-15`
    - `src/query/boolean_query/block_wand_union.rs`

    https://github.com/quickwit-oss/tantivy
14. TigerBeetle docs:
    - `docs/internals/lsm.md` (forest, grooves, incremental compaction)
    - `docs/concepts/safety.md:59-89` (storage fault model, `O_DIRECT`, hash chaining)
    - `docs/TIGER_STYLE.md:151-152` (static allocation)
    - `docs/internals/data_file.md` (checksums stored outside blocks, superblock)

    https://github.com/tigerbeetle/tigerbeetle
15. Datomic documentation: data model (accumulate-only, as-of), index model (EAVT/AEVT/AVET/VAET), excision — https://docs.datomic.com/whatis/data-model.html ; https://docs.datomic.com/indexes/index-model.html ; https://docs.datomic.com/operation/excision.html
16. XTDB README (bitemporal, Arrow columnar, log as coordination; latest release v2.2.0 on 2026-10-21) and SQL transactions (`ERASE`) — https://github.com/xtdb/xtdb ; https://docs.xtdb.com/reference/main/sql/txs.html
17. Pillai et al., "All File Systems Are Not Created Equal: On the Complexity of Crafting Crash-Consistent Applications", OSDI 2014 — https://www.usenix.org/conference/osdi14/technical-sessions/presentation/pillai ; PDF: https://research.cs.wisc.edu/adsl/Publications/alice-osdi14.pdf
18. Rebello et al., "Can Applications Recover from fsync Failures?", USENIX ATC 2020 — https://www.usenix.org/conference/atc20/presentation/rebello
19. PostgreSQL wiki, "Fsync Errors" (fsyncgate 2018, PANIC on failure, Linux 4.13 `errseq_t`, behaviour by OS) — https://wiki.postgresql.org/wiki/Fsync_Errors
20. Corbet, "PostgreSQL's fsync() surprise", LWN, 2018-04-18 — https://lwn.net/Articles/752063/
21. Rust standard library source (main):
    - `library/std/src/sys/fs/unix.rs:1268-1347` (`F_FULLFSYNC` on Apple for both `fsync` and `datasync`; `flock`-based `lock`)
    - `library/std/src/fs.rs:151,822-875` (`file_lock` stable since 1.89.0; docs for `sync_data` and `lock`)

    https://github.com/rust-lang/rust
22. Apple: fsync(2) man page — https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man2/fsync.2.html ; fcntl(2) man page (`F_FULLFSYNC`, `F_NOCACHE`, `F_PREALLOCATE`) — https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man2/fcntl.2.html ; "Reducing disk writes" (`F_BARRIERFSYNC` vs `F_FULLFSYNC`) — https://developer.apple.com/documentation/xcode/reducing-disk-writes
23. Linux man-pages: fsync(2), rename(2), pwritev2(2) (`RWF_ATOMIC` since 6.11), open(2) (`O_DIRECT`) — https://man7.org/linux/man-pages/man2/fsync.2.html ; https://man7.org/linux/man-pages/man2/rename.2.html ; https://man7.org/linux/man-pages/man2/pwritev2.2.html ; https://man7.org/linux/man-pages/man2/open.2.html
24. LevelDB:
    - `doc/log_format.md` (32 KB blocks, crc32c framing)
    - `util/crc32c.h:23-30` (CRC masking)
    - `db/log_format.h:14-16` (zero type reserved for preallocation)
    - `db/log_reader.cc:235-241`

    https://github.com/google/leveldb
25. RocksDB wiki, "Write Ahead Log File Format" (recyclable record format with log number) — https://github.com/facebook/rocksdb/wiki/Write-Ahead-Log-File-Format
26. PostgreSQL docs, WAL configuration (`wal_init_zero`, `wal_recycle`, `wal_sync_method`) — https://www.postgresql.org/docs/current/runtime-config-wal.html
27. Google Security Blog, "Learnings from kCTF VRP's 42 Linux kernel exploits submissions" (2023-06) — https://security.googleblog.com/2023/06/learnings-from-kctf-vrps-42-linux.html
28. moby/moby commit `891241e7e7` "seccomp: block io_uring_* syscalls in default profile" (2023-11-02) — https://github.com/moby/moby/commit/891241e7e7
29. Tokio source: `tokio/src/fs/mod.rs:5-30` (`spawn_blocking`; io_uring only under `tokio_unstable`) and `tokio/src/task/blocking.rs:100-106` (rayon for CPU-bound work) — https://github.com/tokio-rs/tokio
30. Activity of the io_uring runtimes:
    - tokio-uring: https://github.com/tokio-rs/tokio-uring (0.5.0 on 2024-05-27; last commit 2025-07-07)
    - glommio: https://github.com/DataDog/glommio (0.9.0 on 2024-03-25; last main commit 2025-04-21)
    - monoio: https://github.com/monoio-rs/monoio (0.2.4 on 2024-08-20)
31. Mohan et al., "Finding Crash-Consistency Bugs with Bounded Black-Box Crash Testing" (CrashMonkey/ACE), OSDI 2018 — https://www.usenix.org/conference/osdi18/presentation/mohan
32. LazyFS repository — https://github.com/dsrhaslab/lazyfs ; Ramos et al., "When Amnesia Strikes", PVLDB 17(11), 2024 — https://doi.org/10.14778/3681954.3681980
33. Linux kernel docs, dm-log-writes — https://docs.kernel.org/admin-guide/device-mapper/log-writes.html
34. FoundationDB, "Simulation and Testing" — https://apple.github.io/foundationdb/testing.html
35. TigerBeetle, `docs/internals/vopr.md` (deterministic simulation testing; clock, network and disk stubbed) — https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/internals/vopr.md
36. Antithesis, "How Antithesis works" — https://antithesis.com/docs/introduction/how_antithesis_works/
37. madsim README — https://github.com/madsim-rs/madsim
38. turmoil README; `crates/turmoil-fs/README.md`; `crates/turmoil-fs/src/lib.rs:352-446,543-553,647-692,870-883` — https://github.com/tokio-rs/turmoil ; crates https://crates.io/crates/turmoil , https://crates.io/crates/turmoil-fs
39. shuttle README — https://github.com/awslabs/shuttle
40. loom README — https://github.com/tokio-rs/loom
41. stateright README — https://github.com/stateright/stateright
42. cargo-fuzz README (needs nightly; x86-64 and aarch64 Unix) — https://github.com/rust-fuzz/cargo-fuzz
43. proptest and proptest-state-machine — https://crates.io/crates/proptest ; https://crates.io/crates/proptest-state-machine
44. Lemire & Boytsov, "Decoding billions of integers per second through vectorization", Software: Practice and Experience 45(1), 2015 — https://arxiv.org/abs/1209.2137
45. Lemire, Kurz & Rupp, "Stream VByte: Faster Byte-Oriented Integer Compression", Information Processing Letters 130, 2018 — https://arxiv.org/abs/1709.08990
46. Vigna, "Quasi-Succinct Indices", WSDM 2013 — https://arxiv.org/abs/1206.4300
47. Chambi et al., "Better bitmap performance with Roaring bitmaps", Software: Practice and Experience 46(5), 2016 — https://arxiv.org/abs/1402.6407
48. Lemire et al., "Roaring Bitmaps: Implementation of an Optimized Software Library", Software: Practice and Experience 48(4), 2018 — https://arxiv.org/abs/1709.07821
49. Zukowski et al., "Super-Scalar RAM-CPU Cache Compression" (PFOR), ICDE 2006 — https://doi.org/10.1109/ICDE.2006.150
50. Broder et al., "Efficient query evaluation using a two-level retrieval process" (WAND), CIKM 2003 — https://doi.org/10.1145/956863.956944
51. Ding & Suel, "Faster top-k document retrieval using block-max indexes", SIGIR 2011 — https://doi.org/10.1145/2009916.2010048
52. Mallia et al., "Faster BlockMax WAND with Variable-sized Blocks", SIGIR 2017 — https://doi.org/10.1145/3077136.3080780
53. Turtle & Flood, "Query evaluation: strategies and optimizations" (MaxScore), Information Processing & Management, 1995 — https://doi.org/10.1016/0306-4573(95)00020-H
54. Lv & Zhai, "Lower-bounding term frequency normalization" (BM25+), CIKM 2011 — https://doi.org/10.1145/2063576.2063584
55. Robertson, Zaragoza & Taylor, "Simple BM25 extension to multiple weighted fields" (BM25F), CIKM 2004 — https://doi.org/10.1145/1031171.1031181
56. Kamphuis et al., "Which BM25 Do You Mean? A Large-Scale Reproducibility Study of Scoring Variants", ECIR 2020 — https://doi.org/10.1007/978-3-030-45442-5_4
57. GitHub Engineering, "The technology behind GitHub's new code search" (n-gram index) — https://github.blog/engineering/architecture-optimization/the-technology-behind-githubs-new-code-search/
58. Russ Cox, "Regular Expression Matching with a Trigram Index" — https://swtch.com/~rsc/regexp/regexp4.html
59. Elastic docs, `word_delimiter_graph` token filter — https://www.elastic.co/docs/reference/text-analysis/analysis-word-delimiter-graph-tokenfilter
60. Gao & Long, "RaBitQ", PACMMOD 2(3) / SIGMOD 2024 — https://doi.org/10.1145/3654970 ; https://arxiv.org/abs/2405.12497
61. Gao et al., "Practical and Asymptotically Optimal Quantization of High-Dimensional Vectors in Euclidean Space for ANN Search" (Extended RaBitQ), PACMMOD 3(3) / SIGMOD 2025 — https://doi.org/10.1145/3725413 ; https://arxiv.org/abs/2409.09913
62. RaBitQ-Library README and papers page — https://github.com/VectorDB-NTU/RaBitQ-Library ; https://vectordb-ntu.github.io/RaBitQ-Library/papers/
63. Malkov & Yashunin, HNSW — https://arxiv.org/abs/1603.09320
64. "Enhancing HNSW Index for Real-Time Updates: Addressing Unreachable Points and Performance Degradation" (2024) — https://arxiv.org/abs/2407.07871
65. Subramanya et al., "DiskANN: Fast Accurate Billion-point Nearest Neighbor Search on a Single Node", NeurIPS 2019 — https://papers.nips.cc/paper/9527-rand-nsg-fast-accurate-billion-point-nearest-neighbor-search-on-a-single-node.pdf
66. Singh et al., "FreshDiskANN" (2021) — https://arxiv.org/abs/2105.09613
67. "In-Place Updates of a Graph Index for Streaming Approximate Nearest Neighbor Search" (IP-DiskANN, 2025) — https://arxiv.org/abs/2502.13826
68. Chen et al., "SPANN", NeurIPS 2021 — https://arxiv.org/abs/2111.08566
69. Xu et al., "SPFresh: Incremental In-Place Update for Billion-Scale Vector Search", SOSP 2023 — https://doi.org/10.1145/3600006.3613166 ; https://arxiv.org/abs/2410.14452
70. Jégou, Douze & Schmid, "Product Quantization for Nearest Neighbor Search", IEEE TPAMI — https://doi.org/10.1109/TPAMI.2010.57
71. Patel et al., "ACORN", PACMMOD 2(3) / SIGMOD 2024 — https://doi.org/10.1145/3654923 ; https://arxiv.org/abs/2403.04871
72. Gollapudi et al., "Filtered-DiskANN", WWW 2023 — https://doi.org/10.1145/3543507.3583552
73. microsoft/DiskANN (DiskANN3):
    - README
    - `diskann/src/provider.rs:98-200` (`DataProvider` trait)
    - `diskann-quantization/src/lib.rs:11-32` (Spherical/RaBitQ and MinMax quantizers)
    - workspace and crate `Cargo.toml`s
    - crates `diskann*` 0.61.0 (2026-10-26)

    https://github.com/microsoft/DiskANN
74. Rust portable SIMD tracking issue #86656 (open) and `library/core/src/lib.rs:396-397` — https://github.com/rust-lang/rust/issues/86656
75. NumKong (formerly SimSIMD; the GitHub repo redirects) README and `Cargo.toml` (CMake build dependency, `rust-version = "1.100"` on main) — https://github.com/ashvardanian/NumKong ; https://crates.io/crates/simsimd ; https://crates.io/crates/numkong
76. Hugging Face, "Embedding Quantization" blog — https://huggingface.co/blog/embedding-quantization
77. Qdrant docs, Indexing (`full_scan_threshold`) — https://qdrant.tech/documentation/concepts/indexing/
78. Weaviate docs, Vector index concepts (flat, dynamic at 10,000 objects, HFresh) — https://docs.weaviate.io/weaviate/concepts/vector-index
79. Crotty, Leis & Pavlo, "Are You Sure You Want to Use MMAP in Your Database Management System?", CIDR 2022 — https://www.cidrdb.org/cidr2022/papers/p13-crotty.pdf
80. Leis et al., "LeanStore: In-Memory Data Management beyond Main Memory", ICDE 2018 — https://doi.org/10.1109/ICDE.2018.00026
81. Leis et al., "Virtual-Memory Assisted Buffer Management" (vmcache), PACMMOD / SIGMOD 2023 — https://doi.org/10.1145/3588687
82. Kaufmann et al., "Timeline index", SIGMOD 2013 — https://doi.org/10.1145/2463676.2465293
83. Christodoulou, Bouros & Mamoulis, "HINT: A Hierarchical Index for Intervals in Main Memory", SIGMOD 2022 — https://doi.org/10.1145/3514221.3517873
84. RFC 8949, CBOR, §4.2 "Deterministically Encoded CBOR" — https://www.rfc-editor.org/rfc/rfc8949#section-4.2
85. postcard wire format specification (`spec/src/wire-format.md:11,138-160`) — https://github.com/jamesmunns/postcard/blob/main/spec/src/wire-format.md
86. rkyv book, "Feature Comparison" (schema evolution: no) — https://github.com/rkyv/rkyv/blob/main/book_src/feature-comparison.md
87. ciborium `ciborium/src/value/canonical.rs` (main vs tag v0.2.2) — https://github.com/enarx/ciborium
88. RustSec advisory database: zstd 2026-0391/0392/0393; zstd-sys 2026-0410; lz4_flex 2026-0041; memmap2 2026-0186; rkyv 2021-0054, 2026-0001/0122/0233/0234/0235; bincode 2025-0141; capnp 2025-0143; crossbeam-channel 2025-0024; crossbeam-epoch 2026-0204; tokio 2025-0023, 2026-0384, 2026-0386; im 2026-0248 — https://github.com/rustsec/advisory-db/tree/main/crates
89. crates.io API metadata (versions, release dates, licenses, MSRV, downloads) for every crate listed in §2.9 and §2.3 — https://crates.io/api/v1/crates/<name>
90. memmap2 `src/lib.rs:230-235` (`unsafe` constructors; undefined behaviour if the file is modified) — https://github.com/RazrFalcon/memmap2-rs
91. BLAKE3 README and `Cargo.toml:80-91,138-139` (assembly by default, `pure` feature) — https://github.com/BLAKE3-team/BLAKE3
92. CRC crates: crc32c README (Castagnoli, SSE4.2 and aarch64) — https://github.com/zowens/crc32c ; crc32fast README (IEEE) — https://github.com/srijs/rust-crc32fast ; crc-fast README — https://github.com/awesomized/crc-fast-rust
93. roaring-rs README (SIMD via fearless_simd) and `roaring/src/bitmap/serialization.rs:27,61-64` (portable format) — https://github.com/RoaringBitmap/roaring-rs
94. ruzstd README (decoder status, encoder limits) — https://github.com/KillingSpark/zstd-rs ; lz4_flex README (safe-by-default features) — https://github.com/PSeitz/lz4_flex
95. tempfile `src/file/mod.rs:168-177,732-741` (`persist` doesn't sync file or directory) — https://github.com/Stebalien/tempfile
96. fs4 README and `src/unix/sync_impl.rs:9-67` (`allocate` via rustix `fallocate`) — https://github.com/al8n/fs4
97. uuid `src/v7.rs` (v7 feature, `now_v7` ordering) — https://github.com/uuid-rs/uuid
98. Rust 1.62.0 release announcement (futex-based `Mutex` on Linux) — https://blog.rust-lang.org/2022/06/30/Rust-1.62.0/
99. Rust latest release 1.99.1 (2026-10-16) — https://github.com/rust-lang/rust/releases
100. bitpacking README (simdcomp port, 128-integer blocks) — https://github.com/quickwit-oss/bitpacking
101. left-right README (two copies, single writer) — https://github.com/jonhoo/left-right ; arc-swap README — https://github.com/vorner/arc-swap
102. Alagappan et al., "Protocol-Aware Recovery for Consensus-Based Storage", FAST 2018 (background for corruption-aware recovery in v2 replication) — https://www.usenix.org/conference/fast18/presentation/alagappan
103. hivemind research/02, "Storage, indexing, and retrieval foundations" §4 (published measurements of exact kNN scans), in this repository — `research/02-storage-and-retrieval-foundations.md`
