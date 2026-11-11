# Research 02: Storage, indexing, and retrieval foundations

> Research thread report, checked 2026-11-10. Produced by a research agent from public sources and saved verbatim. Evidence labels: **[V]** verified in a primary source, **[C]** project or vendor claim, **[I]** independent evidence, **[Inf]** inference, **[?]** unknown. Synthesis and decisions live in [`docs/`](../docs/).

## Executive summary

- **No single embedded Rust engine covers everything hivemind needs at v1 maturity.** No engine (Lance, Qdrant Edge, Turso, SurrealDB, HelixDB) gives safe multi-process writes, BM25, filtered vectors, light graph and history/versioning in one embedded package. The workable shape is one durable store as the source of truth, with lexical and vector indexes built from it so they can be rebuilt.
- **Exact (brute-force) vector search is enough for most per-scope corpora.** Up to about 100k vectors at full precision, or about 1M with 8-bit or 1-bit compression plus a precise re-check, a laptop can scan them in milliseconds. Published numbers (sqlite-vec: 1M×128 in 17 ms on an M1; LanceDB: 100k×1000-dim under 20 ms) agree with my bandwidth estimates. Approximate indexes become necessary above about 1M vectors per scope, when the data doesn't fit in RAM, or when a shared service needs high query volume (1M×768 full-precision is about 33 queries/s per 100 GB/s of memory bandwidth).
- **Two durability defaults are weaker than they look:**
  - Lance on local disk doesn't fsync by default. A new `local_fsync` option (default off) was merged on 2026-10-28, after a production dataset ended up with empty manifests.
  - Qdrant writes its log through memory mapping and flushes it about every 5 s, unless the client sends `wait=true`. Its docs claim writes survive power loss once they're in the log.
- **SQLite (3.54.1) is still the safest durable core.** It has a multi-process write-ahead log on one machine, FTS5 BM25 and trigram search, and recursive queries. Its sync settings need care: `synchronous=NORMAL` can lose recent commits, and full flushing on macOS is off by default. sqlite-vec came back in March 2026, but its approximate indexes are still alpha and there have been no commits since 2026-05-18.
- **Turso Database (the Rust SQLite rewrite) isn't ready.** It is pre-1.0: multi-process access is experimental, vector search is exact-only, and there are 48 open issues mentioning corruption.
- **Qdrant has the most mature filtered search, hybrid search and multitenancy.** v1.20 added BM25 on its full-text index and a `code` tokenizer. Replication is eventually consistent by default, and Qdrant Edge (the embedded version) is beta.
- **Other engines carry big caveats:**
  - **HelixDB v3** was rebuilt on SlateDB object storage with Tantivy, and it is still young.
  - **SurrealDB 4** ships only in-memory and SurrealKV storage, and its BSL licence forbids offering it as a database service.
  - **Chroma's** hybrid search API is Cloud-only, and its open-source releases have stalled since May 2026.
  - **Kùzu** was archived in October 2025 (the LadybugDB fork is active). **CozoDB** has been dormant since December 2024.
- **Key-value engines:**
  - redb fsyncs by default but added multi-process writers only on 2026-11-05.
  - fjall doesn't fsync by default and is single-process.
  - LMDB/heed is mature, multi-process and single-writer.
  - sled is a stale beta.
- **There is no independent evidence to lean on.** Jepsen has analysed none of these systems, and ann-benchmarks is no longer maintained.

**Candidate stacks:**
- **A:** SQLite + FTS5 + an in-process vector scan, with an approximate index added later.
- **B:** LanceDB embedded.
- **C:** redb or fjall + Tantivy + a vector library.
- **D:** Qdrant (server or Edge) + SQLite.

My recommendation is A as the default, with B and C as contenders. Eight experiments decide between them, most importantly crash tests that simulate lost unsynced writes and scan-versus-approximate tests on real scope-filtered data.

---

## 02 — Storage, indexing, and retrieval foundations for hivemind

*Research thread: building blocks (embedded first, then server). All facts checked 2026-11-10 against public primary sources unless marked otherwise. Labels: **[V]** verified in a primary source, **[C]** project/vendor claim, **[I]** independent third-party evidence, **[Inf]** inference, **[?]** unknown. Citations [Sn] point to the Sources list (§7).*

## 1. Summary

1. **No single embedded Rust engine covers the whole requirement at v1 maturity.** The requirement is durable multi-process appends, BM25, filtered vector search, light graph, and bitemporal history. The defensible v1 is a durable system of record plus indexes built from it that can be rebuilt. **[Inf]**
2. **Exact kNN is feasible per scope.** Brute force works to roughly 100k vectors at f32 and roughly 1M with int8 or binary + rescoring on a modern laptop. Published scans (sqlite-vec on M1: 1M×128 f32 in 17 ms; GIST 500k×960 in 41 ms; LanceDB: 100k×1000-d in under 20 ms) match bandwidth-bound estimates. **[V]/[C]/[Inf]** ([S70], [S30])
3. **ANN is mainly needed in three cases.** A single scope much larger than 1M, data that isn't RAM-resident, or shared-service throughput. Exact scans cost memory bandwidth per query: 1M×768 f32 ≈ 3 GB per query, so about 33 QPS per 100 GB/s. **[Inf]**
4. **SQLite 3.54.1** (bundled by rusqlite 0.41.1) gives WAL multi-process concurrency on one host (one writer at a time), FTS5 BM25/trigram, and contentless-delete. Durability depends on settings: `synchronous=NORMAL` in WAL can roll back commits after power loss, and `fullfsync` (macOS) defaults off. **[V]** ([S63]–[S68])
5. **sqlite-vec** returned in March 2026 (v0.1.7–0.1.9, Mozilla-supported). ANN (rescore, DiskANN; IVF not enabled) exists only in v0.1.10 alphas, and there have been no commits since 2026-05-18. **[V]** ([S69])
6. **Turso Database** (Rust SQLite rewrite, MIT) has MVCC `BEGIN CONCURRENT`, exact vector search, and experimental Tantivy FTS. Its manual still lists "No multi-process access", ANN is "roadmap", and 48 open issues mention "corrupt". Promising, but not a v1 foundation. **[V]** ([S73]–[S75])
7. **LanceDB/Lance** is the strongest embedded all-in-one (MVCC manifests and time travel, BM25 FTS, IVF indexes with SPFresh-style split/join, hybrid RRF in Rust, object-storage path). But local writes are not fsynced by default: `local_fsync` (default off) was merged 2026-10-28 after a production report of zero-length manifests. It is also heavy (79 normal dependencies including DataFusion). **[V]** ([S22]–[S38])
8. **Qdrant v1.20.0** (2026-10-22) is the most mature filtered-HNSW + hybrid engine: dense, sparse, and BM25 on the full-text index, with a new `code` tokenizer, RRF/DBSF/formula fusion, and tiered multitenancy. The WAL is msync'd before apply only when `wait=true`; otherwise it is flushed every `flush_interval_sec: 5`. Replication is availability-first (eventual). **[V]** ([S2], [S7]–[S16])
9. **Qdrant Edge** (embedded, same storage format) is beta (crate 0.9.0). It supports a single writer plus read-only follower processes; crash-replay fixes shipped in v1.20. **[V]** ([S5], [S6], [S17]–[S19])
10. **HelixDB v3** is a new architecture: SlateDB-on-object-storage fork, Tantivy, its own HNSW, Cypher and SQL front ends. It became Apache-2.0 in May 2026 (previously AGPL) and waits for object-store durability by default. Too young for a foundation. **[V]** ([S39]–[S44])
11. **SurrealDB 4.0** (2026-11-03) ships only `kv-mem` and `kv-surrealkv` backends, with HNSW, DiskANN, BM25, graph edges, and an opt-in versioned KV. It syncs on every commit by default on native targets. **BSL 1.1 forbids offering it "as a Database Service"** (change date 2030-01-01). **[V]** ([S51]–[S56])
12. **Chroma** local and single-node mode is Rust bindings + SQLite + an hnswlib fork. The hybrid "Search API is available in Chroma Cloud only." The last PyPI release is 1.5.9 (2026-05-05). **[V]** ([S45]–[S50])
13. **KV engines:**
    - **redb:** stable, crash-safe by default; multi-process `MultiWriter` added in 4.5.0 on 2026-11-05.
    - **fjall:** LSM, flushes to OS buffers only by default; not multi-process.
    - **LMDB/heed:** multi-process, single writer, mature.
    - **RocksDB:** async WAL by default; one primary process.
    - **sled:** "beta", stale.
    - **SlateDB:** object-storage LSM, 0.18.
    All **[V]**. ([S93]–[S101])
14. **Graph engines:**
    - Kùzu was archived 2025-10-10; the LadybugDB fork is active.
    - CozoDB has been dormant since 2024-12.
    - TypeDB 3 is a Rust server on RocksDB; Oxigraph is RDF/SPARQL on RocksDB.
    - Graphiti now marks Kùzu "deprecated" and supports Neo4j, FalkorDB (SSPL), and Neptune.
    For v1-scale "light graph", relational edge tables + recursive CTEs suffice. **[V]/[Inf]** ([S102]–[S112])
15. **No independent durability evidence exists for any candidate** (no Jepsen analyses). ann-benchmarks is unmaintained (points to VIBE). hivemind must run its own crash-injection and retrieval experiments (§5). **[V]** ([S21], [S114], [S115])

---

## 2. Per-candidate findings

Format per candidate: **Claimed** / **Implemented (verified)** / **Demonstrated** / **Maturity** / **Fit**.

### 2.1 Vector and hybrid databases

#### Qdrant (server) — Apache-2.0, Rust
- **Claimed [C]:** "Once a change has been added to the WAL, it will not be lost even if a power loss occurs" ([S11]). It also claims high-performance, massive-scale vector search.
- **Implemented [V]:**
  - **WAL.** `SerdeWal::write` "does [not] guarantee durability" ([S7], wal.rs L101–106). Appends go to an mmap'd segment ([S10], segment.rs L421–447; flush = mmap `flush` ⇒ msync).
  - **Flush timing.** With `wait=true`, the update worker explicitly flushes the WAL before applying ([S8], update_worker.rs L358–364). Otherwise the flush worker runs on `flush_interval_sec: 5` ([S9]). The REST `wait` param is optional ([S123]).
  - **Inference on the durability gap [Inf]:** process crashes are safe (page cache survives). Power loss or OS crash can lose up to about 5 s of acknowledged `wait=false` writes. That contradicts the unqualified doc claim.
  - **Segments and optimizer.** Defaults are `deleted_threshold: 0.2` and `vacuum_min_vector_number: 1000` ([S9]). Deletes are tombstoned and segments are rebuilt by the optimizer **[V]**.
  - **Filterable HNSW.** Payload-aware extra edges (`payload_m`); create payload indexes *before* ingest for the filter-aware links ([S15]). ACORN extended to `m=0` graphs in v1.20 ([S2]).
  - **Hybrid.** Query API with nested `prefetch`, `rrf` (weighted) and `dbsf` fusion ([S14]). v1.20: BM25 scoring directly on the full-text payload index, plus a new `code` tokenizer that splits camelCase/snake_case ([S2], [S15]). Per-query IDF corpus for sparse vectors in v1.19 ([S3]).
  - **Multitenancy.** `is_tenant` keyword index (co-locates tenant data), custom shard keys, and tiered multitenancy with fallback shards ([S16]).
  - **Distributed.** Raft for topology and collection metadata only. Writes run in parallel on replicas. `write_consistency_factor` defaults to 1, read `consistency` defaults to 1, and there are write `ordering` options. Docs say concurrent updates "can result in an inconsistent state" across replicas by default ([S12], [S13]).
- **Demonstrated:**
  - **[V] durability bugs fixed recently:** "optimizer data race… defer source segment destruction until durable" and several WAL replay/data-race fixes in v1.19 ([S3]); WAL data race fixes in v1.17 ([S4]).
  - **[V] issue counts:** 15 open issues match "corrupt"; e.g. #11385 is about the client "local mode" ([S20]).
  - **[I]:** no Jepsen analysis ([S21]). Qdrant appears in the (now unmaintained) ann-benchmarks algorithm set ([S114]). No independent durability study found.
- **Maturity [V]:** v1.20.0 on 2026-10-22; 35.5k stars; daily commits; Docker image about 72.6 MB compressed (amd64) ([S120]).
- **Fit [Inf]:** best-in-class retrieval for a *shared service*. As a laptop daemon it adds a separate process and data dir, and has no time travel. Bitemporal and provenance truth would live elsewhere. Use `wait=true` for acknowledged agent writes.

#### Qdrant Edge — embedded (beta)
- **Claimed [C]:** "Think of it as SQLite, but for vector search"; minimal footprint; no background services ([S5]).
- **Implemented [V]:**
  - Single "Edge Shard" per dataset; no collections. Supports dense, sparse, multivector, BM25, hybrid, quantization, and all payload index types.
  - **Manual `optimize()`:** new points are brute-force searchable until optimized. Snapshots are restore-only. ([S6])
  - WAL on by default. A WAL-less mode where "a crash loses every update since the last flush" ([S17]).
  - v1.20 fixed "updates lost on crash: unacknowledged WAL operations are replayed" and a flush-ack bug ([S2]).
  - Code has a **read-only follower** (`ReadOnlyEdgeShard`): one leader process writes; other processes open the same directory and `live_reload` ([S18]).
- **Demonstrated [V]:** crate 0.9.0, about 25k total downloads ([S19]). Low adoption; no independent evidence.
- **Fit [Inf]:** attractive "same engine local and server" story, but beta, and recent crash-loss fixes signal immaturity. It is single-writer per shard, so a multi-agent laptop needs one owning daemon.

#### LanceDB / Lance — Apache-2.0, Rust (lance-format/lance; lancedb/lancedb)
- **Claimed [C]:** embedded retrieval library; "disk-based brute-force kNN… 100K pairs of 1000-dimension vectors takes under 20ms"; an index is unnecessary at about 100k rows ([S30]).
- **Implemented [V]:**
  - **MVCC and commits.** Each commit writes a new immutable manifest via `rename-if-not-exists` or `put-if-not-exists`. Optimistic concurrency with per-operation rebase rules (Append is compatible with most operations; Delete/Update rebase by merging deletion masks). The version history gives time travel ([S22]).
  - **S3.** Concurrent writers require the store to *enforce* `If-None-Match: *`. "Some S3-compatible stores, including Garage, accept this header but ignore the condition… silently losing acknowledged data"; otherwise configure a commit lock ([S24] L388–397).
  - **Local durability.** Issue #9608: "Lance writes data, transaction and manifest files through object_store's LocalFileSystem, and it does not fsync them." A production dataset had zero-length newest manifests ([S25]). PR #9609 (merged 2026-10-28) adds a `local_fsync` storage option **defaulting to false** ([S26]; documented in [S24] L98–128).
  - **FTS.** Native `INVERTED` index with BM25, phrase, fuzzy, multi-column; N-gram/FM indexes for substring ([S27]).
  - **Hybrid.** Rust `.full_text_search(...).rerank(RRFReranker)` ([S35]).
  - **Vector indexes.** `IVF_PQ`, `IVF_RQ` (RaBitQ), `IVF_HNSW_SQ`, `IVF_HNSW_FLAT` (embedded only). For filtered workloads the docs recommend IVF_RQ/IVF_PQ over HNSW variants ([S30]).
  - **Unindexed rows.** Indexes track a `fragment_bitmap`; unindexed fragments are flat-searched and deleted rows are filtered post-search ([S28]). `optimize()` = compaction + cleanup (7-day retention default) + incremental index update ([S31]). IVF partitions split/join on incremental updates (`test_spfresh_join_split`, [S29]).
  - **Freshness.** Embedded handles don't see other processes' commits unless `read_consistency_interval` or `checkout_latest()` is used ([S32]).
  - **New "Real Time Engine" (RTE)** on the experimental **MemWAL** LSM spec: memtable + WAL + SSTables, available embedded. Not used for time-travel or hybrid queries (must `use_rte(False)`) ([S23], [S33], [S34]).
- **Demonstrated:**
  - **[I]:** Lance file-format paper (arXiv 2504.15247; authored by the project, peer status [?]) ([S37]).
  - **[V] open issues:** #3377 "mergeInsert silently produces duplicate rows under concurrent writers… OCC conflict detection appears to never fire on the JS binding path" ([S36]); #2423 on crash during `optimize()`.
- **Maturity [V]:** lance 14.0.2 stable (2026-10-29) with v15 betas daily; lancedb crate 0.41.0. Heavy dependency tree (Arrow, DataFusion, object_store; 79 normal deps) ([S38]).
- **Fit [Inf]:**
  - **Strengths:** versioned history maps well to *record time*; hybrid retrieval is built in; object storage gives a path to shared deployments.
  - **Risks:** durability opt-in, small-commit amplification without RTE, maintenance chores (optimize/cleanup), and build weight.

#### HelixDB (as a database) — Apache-2.0 (since 2026-05), Rust
- **Claimed [C]:** "OLTP graph database with native vector and full-text search built in Rust on object storage"; transactional ([S39]).
- **Implemented [V]:**
  - v3 storage is "One owned SlateDB transaction" → "SlateDB / object storage" ([S40] L205–237). It uses a git fork `HelixDB/slatedb`, `tantivy 0.26` for text, and its own `search/vector/hnsw` ([S41]).
  - Front ends: native JSON AST, Cypher, and SQL translated to Cypher ([S40]). The HelixQL compile step is gone ("no build or deploy step") ([S39]).
  - Commits wait for object-store durability unless the client sends `x-helix-await-durable: false` ([S42], [S43]).
  - Local dev data is in-memory unless started with `--disk`; an embedded mode exists **[C]** ([S39]).
- **Demonstrated:** none independent. Rewrite churn: v2.0.0 on 2025-09-27, v3.0.0 on 2026-05-15, now v3.6.2; licence moved from AGPL to Apache (commit 2026-05-28) **[V]** ([S44]).
- **Fit [Inf]:** a conceptually close match (graph + vector + BM25 + transactions). But the engine is about six months into a re-architecture on a forked, pre-1.0 SlateDB. Watch list.

#### Chroma — Apache-2.0
- **Implemented [V]:**
  - Three modes: local (embedded), single-node, distributed ([S45]).
  - Single-node uses "a fork of hnswlib" that must be RAM-resident, plus SQLite for documents and metadata ([S46]). Python bindings are a Rust crate (`chromadb_rust_bindings`) over `chroma-frontend` + `sqlx` SQLite ([S50]).
  - Local FTS is a filter (`$contains`, `$regex`, case-sensitive), not BM25 ranking ([S48]).
  - **"Search API is available in Chroma Cloud only"** (hybrid/RRF) ([S47]).
  - Distributed uses SPANN in Rust (`rust/index/src/spann`) with object storage ([S92]).
- **Demonstrated [C]:** vendor sizing table, e.g. t3.small (2 GB) holds about 250k 1024-d vectors ([S46]).
- **Maturity [V]:** PyPI 1.5.9 on 2026-05-05 (no newer release). Commits fell from about 90/month (May 2026) to 16 (October 2026) ([S49]).
- **Fit [Inf]:** fine for prototypes. The OSS hybrid gap and slowing cadence make it a poor foundation.

#### SurrealDB 4 (embedded) — BSL 1.1
- **Implemented [V]:**
  - Backends are only `kv-mem` and `kv-surrealkv` (RocksDB config keys are listed as removed) ([S53], [S52]).
  - SurrealKV is now an LSM with an async commit pipeline, OCC, snapshot isolation, optional versioned reads (`get_at`, history), and value-log separation ([S56]).
  - The library defaults to `Durability::Eventual` ([S56] L191), **but SurrealDB sets `SyncMode::Every` on native targets** (1 s interval in WASM). `versioned: false` by default ([S52] L15–17, 81–112).
  - Indexes: `FULLTEXT … BM25`, `HNSW`, `DISKANN`, `COUNT`, `SPATIAL`, with `CONCURRENTLY` builds (MTREE is not in the 4.x syntax) ([S54]).
  - Licence: "you may not use the Licensed Work as a Database Service", change date 2030-01-01 → Apache-2.0 ([S51]).
- **Demonstrated [C]:** SurrealKV README benchmarks against RocksDB/SlateDB (vendor, single runs) ([S56]).
- **Maturity [V]:** v4.0.0 on 2026-11-03, v4.0.1 on 2026-11-09. Brand-new major version.
- **Fit [Inf]:** feature-rich (graph + vector + FTS + versioning). The BSL restriction and a one-week-old major version argue against it as a foundation.

#### Meilisearch — MIT AND BUSL-1.1 (enterprise parts)
- **Implemented [V]:**
  - LMDB via `heed 0.23`; vectors via `hannoy 0.3` (LMDB-backed HNSW with roaring-bitmap edges, dynamic insert/delete), with `arroy 0.9` kept for migration ([S58], [S60]). hannoy migration landed in v1.22.0 (2025-09-24) ([S59]).
  - Writes are asynchronous tasks auto-batched in a queue ([S61]). Hybrid uses `semanticRatio` ([S62]). Licence split per [S57].
- **Fit [Inf]:** a solid hybrid search *server*, but not embeddable (the `milli` crate on crates.io is a 2020 stub) and eventually visible. hannoy itself is a reusable building block: an LMDB-backed, multi-process-readable HNSW.

### 2.2 SQL / embedded

#### SQLite (rusqlite) + FTS5 (+ sqlite-vec)
- **Implemented [V]:**
  - SQLite 3.54.1 (2026-10-16); rusqlite 0.41.1 bundles 3.54.1 ([S67], [S68]).
  - **WAL:** readers and writer don't block each other. "All processes using a database must be on the same host… WAL does not work over a network filesystem." Auto-checkpoint at 1000 pages; checkpoint starvation if readers are always open ([S63]).
  - **Durability:** "A transaction committed in WAL mode with synchronous=NORMAL might roll back following a power loss"; it is durable across application crashes regardless ([S64]). `fullfsync` / `checkpoint_fullfsync` default off (macOS only) ([S64]).
  - **Concurrent writers:** `BEGIN CONCURRENT` exists only on a branch ([S66]).
  - **FTS5:** `bm25()`, trigram tokenizer, external-content/contentless tables, contentless-delete (since 3.43.0), `secure-delete` ([S65]).
- **sqlite-vec [V]:**
  - v0.1.7 "is back! After a long hiatus" (2026-03-17; Mozilla support); stable v0.1.9 (2026-03-31).
  - v0.1.10-alpha.1–.4 add "rescore, ivf (experimental, not enabled), and DiskANN". DiskANN deletes are "quite expensive" ([S69]).
  - Last commit 2026-05-18; 7 open issues match "corrupt" (keyword match, unreviewed).
- **Demonstrated [C, single author, M1 Mac mini, in-RAM]:**
  - sift1m (1M×128) k=20: sqlite-vec static 17 ms, vec0 33–35 ms, faiss 10 ms, usearch exact 56 ms, numpy 136 ms.
  - GIST 500k×960: static 41 ms, usearch exact 46 ms, faiss 50 ms, vec0 87–89 ms, duckdb 307 ms.
  - On-disk vec0: at 100k, 768-d float and smaller come in under 75 ms; at 1M, float takes 192 ms (192-d) to 8.52 s (3072-d), and the best binary result is 124 ms ([S70]).
- **Fit [Inf]:** the most battle-tested durability and the smallest footprint. Vectors can live as BLOB columns scanned by app-side SIMD, so sqlite-vec isn't required.

#### libSQL — MIT
- **Implemented [V]:** fork of SQLite; "inherits SQLite's fundamental limitations such as the single-writer model"; "new features are being developed in Turso" ([S71]). Native vector types and functions, plus a DiskANN index via `libsql_vector_idx` / `vector_top_k`, with tunables (`max_neighbors`, `compress_neighbors`, `alpha`). The index needs a ROWID or single-column PK ([S72]).
- **Maturity [V]:** `libsql` crate 0.10.0 (2026-10-13); last server release February 2025.
- **Fit [Inf]:** the only SQLite-family engine with an in-engine graph ANN today, but its owner is steering users to Turso Database.

#### Turso Database (Rust rewrite) — MIT
- **Implemented [V]:**
  - `BEGIN CONCURRENT` (MVCC, row-level conflict detection at commit), change data capture (CDC), vector types `vector32`/`vector8`/`vector1bit`/sparse with **exact search**.
  - "Vector indexing for fast approximate vector search" is **roadmap**.
  - Experimental: Tantivy-backed FTS, encryption, Postgres front end, multi-process WAL via `.tshm` ([S73]).
  - Manual limitations: "No multi-process access", "No multi-threading", no triggers/views/vacuum. MVCC caveats include a blocking `TRUNCATE` checkpoint ([S74] L96–118).
- **Demonstrated [C]:** production users named; DST + Antithesis testing; DBTest '26 paper on its random testing ([S73]). **[V]** 48 open issues match "corrupt", many found by simulators/fuzzers ([S75]).
- **Fit [Inf]:** revisit at 1.0. Not a v1 base.

#### DuckDB 2.0 LTS (+ VSS, FTS) — MIT
- **Implemented [V]:**
  - "Read-write mode: one process"; multi-process only in read-only mode ([S76]).
  - VSS HNSW (usearch-based): persistence needs `hnsw_enable_experimental_persistence` because "WAL recovery is not yet properly implemented… data loss or corruption of the index". The whole index is re-serialized on each checkpoint, must fit in RAM, and deletes are marked until `PRAGMA hnsw_compact_index` ([S77], [S78]).
  - FTS `match_bm25`; the index doesn't auto-update unless `incremental = 1` ([S79]).
- **Fit [Inf]:** a great *analytics* sidecar (e.g. over exported memory logs). Wrong write model and vector durability for the primary store.

### 2.3 Search and ANN libraries

| Library | Verified facts | Fit |
|---|---|---|
| **Tantivy** (MIT) | Immutable segments; commit writes segments then atomically updates `meta.json` ([S81]). Deletes by term → alive-bitset per commit ([S81]). Single writer enforced by `.tantivy-writer.lock`; `META_LOCK` lets *other processes* read safely ([S82]). Writer ≥ 15 MB/thread, ≤ 8 threads ([S83]). Commits fsync via `Directory::sync_directory` **[V]** (trait). 0.27.0 on crates.io (2026-10-17) ([S84]). Used inside HelixDB and Turso FTS **[V]**. | Best Rust BM25. Commit cost argues for batching (sub-second to seconds) **[Inf]**. |
| **USearch** (Apache-2.0) | Stable 2.26.6 crate: `add/remove/search/filtered_search/exact_search/view` (mmap)/`save/load` ([S86]). **`main` is `3.0.0-rc.1`, a large API rewrite** (Executor/Arena, NumKong kernels, NVMe residency) ([S85]). README says Chroma uses it behind its SPANN index **[C]**. | Capable; plan for v2→v3 API churn **[Inf]**. |
| **hnsw_rs** (MIT/Apache) | Multithreaded insert/search; dump/reload; filter during search; mmap on data (not graph); **no delete API documented** ([S87]). | OK for append-only. |
| **instant-distance** | **Archived** (djc/instant-distance); last crate release 0.6.1 in 2023 ([S88]). | Avoid. |
| **arroy / hannoy** (MIT) | LMDB-backed ANN; hannoy = HNSW with roaring edges (~200 B/vector), incremental insert/delete ([S60]). | Good if LMDB is the base. |
| **DiskANN3** (microsoft/DiskANN, MIT, Rust) | "Composable library": implement a `DataProvider` trait for your store. Real-time updates via IP-DiskANN/Fresh-DiskANN "no merges, rebuilds"; filter hooks. Old C++ on `cpp_main` ([S89]). Crate 0.61.0. | Interesting for an ANN *inside* hivemind's own KV. Young API **[Inf]**. |
| **SPFresh-style incremental ANN** | SPFresh paper (in-place updates; [S90]); reference C++ repo idle since 2025-05 ([S91]). Rust adopters: Chroma SPANN ([S92]); Lance IVF split/join ([S29]). | Relevant only at multi-million scale **[Inf]**. |

### 2.4 KV engines

| Engine | Concurrency / processes | Durability default | Status |
|---|---|---|---|
| **redb** (Apache-2.0) | MVCC, one writer + many readers. 4.5.0 (2026-11-05) adds `ConcurrencyMode::{ExclusiveWriter (default), SingleWriter, MultiWriter}` for multi-process ([S94]). | "Crash-safe by default" ([S93]). | Stable file format; very active. MultiWriter is 5 days old. |
| **fjall** (Apache-2.0) | Thread-safe; "may **not** be loaded in parallel from separate processes" ([S95]). | Flushes to OS buffers, **not** disk, by default; `persist(SyncAll)` to fsync ([S95]). | 3.1.13 (2026-10-25). |
| **LMDB / heed** | Multi-process + multi-thread; "only one write transaction may be active at a time"; readers lock-free ([S96]). | Copy-on-write; no recovery needed ([S96]). | heed 0.23.1; used by Meilisearch ([S97]). |
| **RocksDB** (rust-rocksdb 0.25) | One primary; read-only/secondary instances in other processes ([S99]). | Writes async by default; `sync=true` per write ([S98]). | Mature (C++); heavy build. |
| **sled** | — | — | README: "if reliability is your primary constraint, use SQLite. sled is beta"; main branch mid-rewrite ([S100]). Avoid. |
| **SlateDB** (Apache-2.0) | Embedded LSM on object storage; single writer **[V/Inf]**. | `put` returns after the in-memory WAL; call `await_durable()` or `flush()` ([S101]). | 0.18.1 (2026-11-07); HelixDB depends on a fork. |
| **SurrealKV** (Apache-2.0) | OCC, snapshot isolation, optional versions ([S56]). | `Durability::Eventual` by default (library) ([S56]). | 1.3.0 (2026-11-06). |

**Platform note [V]:** Rust `File::sync_all` on Apple platforms issues `F_FULLFSYNC` ([S117]). SQLite does not unless `PRAGMA fullfsync=ON` ([S64]). mmap/msync-based WALs (Qdrant, LMDB) don't get `F_FULLFSYNC` from msync on macOS **[Inf]**.

### 2.5 Graph engines

| Engine | Status (checked 2026-11-10) |
|---|---|
| **Kùzu** | **Archived 2025-10-10**; "Kuzu is working on something new"; last release 0.11.3 ([S103]). Forks: **LadybugDB** active (v0.22.3, 2026-11-07; crate `lbug`) ([S104]); **Bighorn** archived 2026-08 ([S105]); **RyuGraph** last push 2026-01 ([S106]). |
| **CozoDB** (MPL-2.0) | Calls itself "The hippocampus for AI!" Last release 0.7.6 (2023-12-11), last push 2024-12-04 → dormant ([S102]). |
| **TypeDB 3.x** (MPL-2.0) | Rust server on RocksDB; 3.15.0-rc0 (2026-11-10) ([S107]). Server-only; no vectors **[Inf]**. |
| **Oxigraph** (Apache/MIT) | SPARQL DB "based on the RocksDB key-value store"; 0.5.13 (2026-10-22) ([S108]). |
| **IndraDB** (MPL-2.0) | TAO-inspired; pluggable RocksDB/memory stores; v5.0.0 (2025-08-16), no pushes since ([S109]). |
| **Graphiti backends** | Neo4j 5.26, FalkorDB 1.1.2 (+ embedded `falkordblite`), Neptune + OpenSearch; **Kuzu deprecated**; also promotes Zep's "Konig" service ([S110], [S112]). FalkorDB is **SSPL** ([S111]). |

**[Inf]** hivemind's graph needs are entity–fact–evidence links and supersession chains, mostly 1–3 hop traversals per query. Those are well served by edge tables plus indexes and recursive CTEs (SQLite) or adjacency keys (KV). A graph engine is not justified for v1.

---

## 3. Comparison tables

### 3.1 Retrieval engines and databases

| System | Embedded / server | Multi-process | Concurrency | Durability default | Search modes | Insert/update/delete cost | Temporal / versioning | License | Latest (date) |
|---|---|---|---|---|---|---|---|---|---|
| Qdrant server | Server | n/a (client-server) | Parallel updates; replicas eventual by default | WAL msync on `wait=true`; else ~5 s flush | Dense, sparse, BM25 (v1.20), filters, RRF/DBSF/formula | Tombstones + background optimizer | Snapshots only | Apache-2.0 | v1.20.0 (2026-10-22) |
| Qdrant Edge | Embedded (beta) | 1 writer + RO followers | Single shard | WAL on (opt-out) | Same as server | Manual `optimize()`; brute force until then | None | Apache-2.0 | crate 0.9.0 (2026-10-22) |
| LanceDB / Lance | Embedded (+ Enterprise server) | Yes, via OCC commits | MVCC, optimistic | **No fsync local unless `local_fsync=true`** | Vector (IVF*), BM25, scalar, hybrid RRF | Append-cheap; deletion vectors; `optimize()` folds deltas | **Versions / time travel**, tags/branches | Apache-2.0 | lance 14.0.2 / v15 beta (2026-11-10) |
| HelixDB v3 | Server + embedded **[C]** | [?] | SlateDB txn | Waits object-store durable | Graph + HNSW + BM25 (Tantivy) | [?] | [?] | Apache-2.0 | v3.6.2 (2026-11-10) |
| Chroma local/single-node | Both | [?] (not documented) | — | SQLite + HNSW files | Vector + metadata/regex filters (no BM25 locally) | HNSW in RAM | None | Apache-2.0 | 1.5.9 (2026-05-05) |
| SurrealDB 4 | Both | [?] | OCC / SI | Sync every commit (native) | HNSW, DiskANN, BM25, graph | [?] | Opt-in versioned KV | **BSL 1.1** | v4.0.1 (2026-11-09) |
| Meilisearch | Server | n/a | LMDB single writer; async tasks | LMDB commit | BM25-like ranking + hannoy HNSW hybrid | Batched tasks | None | MIT + BUSL | v1.56.2 (2026-11-09) |

### 3.2 SQL and embedded stores

| System | Multi-process | Writers | Durability default | Lexical | Vector | Versioning | License | Latest |
|---|---|---|---|---|---|---|---|---|
| SQLite (WAL) | Yes, same host | 1 at a time | Depends on `synchronous`; macOS `fullfsync` off | FTS5 BM25, trigram | App BLOB scan / sqlite-vec | None (app columns) | Public domain | 3.54.1 (2026-10-16) |
| sqlite-vec | (SQLite) | (SQLite) | (SQLite) | — | Flat; ANN alpha | — | Apache/MIT | 0.1.9 / 0.1.10-α4 (2026-05-18) |
| libSQL | Yes (SQLite) | 1 | SQLite | FTS5 | DiskANN index | None | MIT | crate 0.10.0 (2026-10-13) |
| Turso DB | **No** (experimental `.tshm`) | MVCC concurrent | [?] | Tantivy FTS (experimental) | Exact only | CDC | MIT | 0.8.3 / 0.9-pre (2026-11-10) |
| DuckDB 2.0 | RO only | 1 process, many threads | WAL | FTS ext | VSS HNSW (experimental persistence) | None | MIT | v2.0.0 (2026-10-21) |

### 3.3 Libraries and KV engines

Covered in §2.3 and §2.4. Summary of maturity rank **[Inf]**:
- **Stable:** LMDB/heed, RocksDB, redb (single-process), Tantivy, SQLite.
- **Usable with care:** fjall, usearch 2.x, hnsw_rs, hannoy.
- **Young or churning:** redb MultiWriter, usearch 3, DiskANN3 Rust, SlateDB, SurrealKV LSM, Lance MemWAL.
- **Avoid:** sled, instant-distance, CozoDB, Kùzu (original).

---

## 4. Brute-force kNN feasibility

### 4.1 Published data points

**sqlite-vec (author's benchmark, M1 Mac mini 8 GB, single query, in-RAM)** [C] ([S70]):
- 1M×128 f32: 17 ms (static) and 33 ms (vec0).
- 500k×960 f32: 41 ms (static), 46 ms (usearch exact), 50 ms (faiss).
- Implied effective scan bandwidth ≈ 30–47 GB/s **[Inf]**.
- On-disk vec0 at 1M: ≥ 192 ms for float, 124 ms for the best binary case. Disk/SQLite paging dominates.

**LanceDB** [C] ([S30]): 100k×1000-d f32 exact < 20 ms, i.e. > 20 GB/s effective.

**Hugging Face embedding-quantization study** [I/C: HF + mixedbread] ([S113]). Exact CPU search, 1024-d, USearch (int8) and Faiss (binary):
- int8 mean **3.66×** speedup (2.99–4.8×); binary mean **24.76×** (15–45.8×).
- Binary + float rescoring keeps about **96%** of retrieval quality (about 92.5% without rescoring).
- int8 with rescore multiplier 4–5 keeps about **99%**.

### 4.2 First-principles estimates [Inf]

**Assumptions:**
- Vectors are RAM-resident and stored contiguously **per scope**.
- The scan is memory-bandwidth-bound.
- About 20 GB/s for one core (conservative x86 laptop) and about 100 GB/s across all cores.
- Current MacBook Pro chips list 153–614 GB/s peak ([S118]); dual-channel DDR5-5600 is 89.6 GB/s theoretical.
- Binary Hamming on AVX2 can be compute-bound at roughly 10–30 GB/s per core.

| N | dim | f32 size → 1 core / all cores | int8 size → 1 core / all | binary size → 1 core / all |
|---|---|---|---|---|
| 10k | 384 | 15 MB → 0.8 / 0.2 ms | 3.8 MB → 0.2 / <0.1 ms | 0.5 MB → <0.1 ms |
| 10k | 768 | 31 MB → 1.5 / 0.3 ms | 7.7 MB → 0.4 / 0.1 ms | 1 MB → <0.1 ms |
| 100k | 384 | 154 MB → 7.7 / 1.5 ms | 38 MB → 1.9 / 0.4 ms | 4.8 MB → 0.2–0.5 ms |
| 100k | 768 | 307 MB → 15 / 3 ms | 77 MB → 3.8 / 0.8 ms | 9.6 MB → 0.5–1 ms |
| 1M | 384 | 1.5 GB → 77 / 15 ms | 384 MB → 19 / 4 ms | 48 MB → 2–5 / 0.5–1 ms |
| 1M | 768 | 3.1 GB → 154 / 31 ms | 768 MB → 38 / 8 ms | 96 MB → 5–10 / 1–2 ms |

- **Rescoring** the top `k×(4–10)` candidates with f32 is microseconds if f32 is in RAM. If f32 lives on SSD: 40–400 random 3 KB reads ≈ 3–30 ms at queue depth 1, under 2 ms with parallel I/O.
- **Throughput ceiling** in shared mode is about (bandwidth ÷ bytes scanned per query). At 100 GB/s:
  - 1M×768: f32 ≈ 33 QPS, int8 ≈ 130 QPS, binary ≈ 1,000 QPS.
  - 100k×768 f32 ≈ 325 QPS.
- **Filters help brute force but hurt ANN.** A scope/project/time filter shrinks the scan when data is laid out per scope, and results stay exact under any selectivity. Filtered HNSW needs special machinery (Qdrant payload links/ACORN [S15], [S2]) and stays approximate (DuckDB: "Filtered search remains approximate" [S78]).

### 4.3 When ANN becomes necessary [Inf]

1. **Single-user laptop, interactive budget (~20–50 ms):**
   - f32 is fine to about 100–300k at 768-d.
   - int8 is fine to about 1M.
   - Binary + rescore is fine to several million.
2. **Shared service:** when aggregate QPS × bytes per query approaches node bandwidth. For example, 50 agents × 1 q/s on a 1M×768 f32 scope ≈ 150 GB/s, which exceeds one node.
3. **Data not RAM-resident:** SSD scans at roughly 3–7 GB/s make 1M×768 f32 about 0.5–1 s. Keep a binary/int8 tier in RAM or use DiskANN-style indexes.
4. **Corpora ≫ 1M per scope.** Then IVF+RaBitQ (Lance), HNSW (Qdrant/hannoy/usearch), or DiskANN3.

**Implication:** v1 can start exact-only (int8/binary + f32 rescoring) behind an interface that lets an ANN index be added per large scope.

---

## 5. Candidate stacks and deciding experiments

### Stack A — "SQLite core" (recommended default to evaluate first)
**Shape:**
- rusqlite (bundled 3.54.x), WAL, `synchronous=FULL` (and `fullfsync` on macOS); or NORMAL plus explicit durability points.
- Facts, evidence, entities and edges as tables with bitemporal columns (valid_from/valid_to, recorded_at, superseded_by).
- FTS5 external-content (unicode61 + optional trigram) for BM25.
- Embeddings as BLOB columns, int8/binary in RAM caches, scanned with SIMD in-process.
- RRF fusion in application code.
- Optional later ANN sidecar (usearch 2.x / hannoy / DiskANN3 provider), always rebuildable.

**Local-lean.** Pros: one file per scope, minimal RAM and fast startup **[Inf]**, best durability record. Cons: single writer means agents should go through one daemon or tolerate `SQLITE_BUSY` waits.

**Shared-service.** Pros: shard by scope across files. Cons: no built-in replication (backup/replication tooling not evaluated here [?]).

**Risks:** FTS5 on code identifiers (trigram index size), app-built hybrid ranking, sqlite-vec stalling.

### Stack B — "LanceDB embedded"
**Shape:** LanceDB Rust crate with `local_fsync=true`. Versions serve as record time; FTS + IVF + RRF; RTE/MemWAL for small appends; S3 or Enterprise for shared mode.

**Pros:**
- Built-in time travel and hybrid search.
- Incremental IVF with split/join.
- Path to object storage.

**Cons and risks:**
- Durability is opt-in and new.
- Commit-per-write amplification and optimize/cleanup chores.
- Multi-process OCC edge cases (#3377).
- 79-dependency build.
- Rapid major-version churn (v14 → v15).

### Stack C — "Rust KV + derived indexes"
**Shape:**
- redb (or LMDB/heed for mature multi-process; fjall for LSM write throughput) holds an append-only event log and current-state tables.
- Tantivy holds BM25 (custom code tokenizer).
- Vectors: exact scan → usearch 2.x / hnsw_rs / DiskANN3 provider over the KV.
- All indexes are keyed by log sequence number and rebuilt idempotently.

**Pros:** pure Rust; full control of the bitemporal/provenance model; small dependency surface.

**Cons:**
- No atomic commit across KV + Tantivy + vector index, so an outbox/replay protocol is needed.
- Tantivy single writer and commit latency.
- Most engineering effort; redb MultiWriter is unproven.

### Stack D — "Qdrant + relational system of record"
**Shape:** Qdrant server (shared) or Edge (local) for retrieval (dense + sparse + BM25 with `code` tokenizer, filters, tenants). SQLite or Postgres for facts, provenance, graph and history.

**Pros:**
- Mature filtered HNSW and one-query hybrid.
- Replication and tiered multitenancy.

**Cons:**
- Dual-write consistency.
- Edge is beta.
- Async WAL unless `wait=true`.
- Extra process and image (~73 MB).
- No time travel.

**Watch list (not v1 foundations):** Turso DB (re-test at 1.0), HelixDB v3, SurrealDB 4 (BSL), sqlite-vec ANN, Lance MemWAL, DiskANN3. Postgres + pgvector (v0.8.7 tag) + ParadeDB (AGPL-3.0, v0.26.6) is a shared-service baseline not evaluated in depth here ([S122]).

### Deciding experiments
- **E1 — Exact vs ANN on real memory data.** 10k/100k/1M × {384, 768} × {f32, int8, binary + rescore} vs HNSW/IVF. Scope filters at 0.1%, 1%, 10% and 100% selectivity. Measure recall@10, p50/p99 latency, and QPS ceiling on an Apple-silicon and an x86 laptop.
- **E2 — Crash and power-loss injection.** Use kill -9 plus LazyFS ([S119]) to drop unsynced writes. Targets:
  - SQLite with FULL vs NORMAL, and fullfsync on/off.
  - redb.
  - fjall with and without `persist`.
  - Lance with `local_fsync` off/on.
  - Qdrant with `wait` false/true.
  - Qdrant Edge with WAL.
  - Tantivy commit.

  Check that every acknowledged write survives and the store reopens cleanly.
- **E3 — Multi-process contention.** 4–16 MCP-style processes appending and querying at once. Compare SQLite WAL + busy_timeout, redb SingleWriter/MultiWriter, Lance OCC, Edge leader/follower. Record p99 write latency and errors.
- **E4 — Trickle-append amplification.** One item per second for 8 hours plus 10% supersession updates. Measure disk growth, version and fragment count, and segment merges; also optimize/compaction CPU and the effect on query latency.
- **E5 — Lexical quality on agent memory and code.** FTS5 unicode61 vs trigram vs Tantivy (custom tokenizer) vs Qdrant `code` tokenizer, each with hybrid RRF, on a labelled retrieval set (benchmarks covered by other threads).
- **E6 — Footprint.** Binary size, cold start, idle RSS, and RSS at 100k items for each stack embedded in a minimal Rust daemon.
- **E7 — Rebuild time.** Rebuild FTS and vector indexes from the system of record at 100k and 1M items. This sets how practical the "derived index" design is.
- **E8 — Churn.** Delete or supersede 20–50% of items. Measure ANN recall drift and the cost of tombstone cleanup (Qdrant vacuum, Lance deletion vectors, usearch remove, hnsw_rs rebuild).

---

## 6. Open questions

1. **Embedding model and dimension per scope** (384 vs 768 vs Matryoshka truncation) and whether binary-trained models are acceptable. This dominates the brute-force math.
2. **Real distribution of scope sizes.** If 95% of scopes are ≤ 100k items, ANN can be deferred entirely.
3. **Write durability contract.** Is "acknowledged ⇒ survives power loss" required, or "survives process crash" enough (SQLite NORMAL, Qdrant `wait=false`, fjall defaults)?
4. **Process model.** Is there one owning daemon per machine, or do several agent processes open the store directly? This decides whether multi-process write support (redb MultiWriter, SQLite WAL, Lance OCC) matters.
5. **Licence policy.** Are BSL (SurrealDB, Meilisearch EE), SSPL (FalkorDB) and AGPL (ParadeDB) acceptable for a shared-service deployment?
6. **Bitemporal semantics.** Engine versioning (Lance versions, SurrealKV `get_at`) captures record time only. Valid time must be modelled explicitly. Is engine time travel still worth its cost?
7. **Object storage in v1.** Lance, SlateDB and HelixDB all assume it for scale-out. Is it needed before the shared-service milestone?
8. **macOS durability.** Should hivemind standardize on `F_FULLFSYNC` semantics (automatic via Rust std; opt-in for SQLite)?
9. **Unverified items [?]:** Chroma and SurrealDB embedded multi-process behaviour; HelixDB embedded durability and versioning; idle RSS and startup numbers (no primary sources found); Turso durability defaults.

---

## 7. Sources
(all checked 2026-11-10)

1. [S1] https://github.com/qdrant/qdrant
2. [S2] https://github.com/qdrant/qdrant/releases/tag/v1.20.0
3. [S3] https://github.com/qdrant/qdrant/releases/tag/v1.19.0
4. [S4] https://github.com/qdrant/qdrant/releases/tag/v1.17.0
5. [S5] https://qdrant.tech/documentation/edge/
6. [S6] https://qdrant.tech/documentation/edge/edge-vs-qdrant-cluster/
7. [S7] https://github.com/qdrant/qdrant/blob/222b2e44ab1c3f17e8246396ead6558e45041df7/lib/shard/src/wal.rs#L101-L106
8. [S8] https://github.com/qdrant/qdrant/blob/222b2e44ab1c3f17e8246396ead6558e45041df7/lib/collection/src/update_workers/update_worker.rs#L358-L364
9. [S9] https://github.com/qdrant/qdrant/blob/222b2e44ab1c3f17e8246396ead6558e45041df7/config/config.yaml#L190-L242
10. [S10] https://github.com/qdrant/qdrant/blob/222b2e44ab1c3f17e8246396ead6558e45041df7/lib/wal/src/segment.rs#L421-L447
11. [S11] https://qdrant.tech/documentation/manage-data/storage/
12. [S12] https://qdrant.tech/documentation/scaling/consistency-guarantees/
13. [S13] https://qdrant.tech/documentation/operations/distributed_deployment/
14. [S14] https://qdrant.tech/documentation/search/hybrid-queries/
15. [S15] https://qdrant.tech/documentation/manage-data/indexing/
16. [S16] https://qdrant.tech/documentation/manage-data/multitenancy/
17. [S17] https://github.com/qdrant/qdrant/blob/222b2e44ab1c3f17e8246396ead6558e45041df7/lib/edge/ffi/src/config.rs#L1016-L1021
18. [S18] https://github.com/qdrant/qdrant/blob/222b2e44ab1c3f17e8246396ead6558e45041df7/lib/edge/src/read_only/mod.rs#L1-L13
19. [S19] https://crates.io/crates/qdrant-edge
20. [S20] https://github.com/qdrant/qdrant/issues/11385
21. [S21] https://jepsen.io/analyses
22. [S22] https://github.com/lance-format/lance/blob/8f07c195f357/docs/src/format/table/transaction.md
23. [S23] https://github.com/lance-format/lance/blob/8f07c195f357/docs/src/format/table/mem_wal.md
24. [S24] https://github.com/lance-format/lance/blob/8f07c195f357/docs/src/guide/object_store.md
25. [S25] https://github.com/lance-format/lance/issues/9608
26. [S26] https://github.com/lance-format/lance/pull/9609
27. [S27] https://github.com/lance-format/lance/blob/8f07c195f357/docs/src/guide/full_text_search.md
28. [S28] https://github.com/lance-format/lance/blob/8f07c195f357/docs/src/format/index/index.md
29. [S29] https://github.com/lance-format/lance/blob/8f07c195f357/rust/lance/src/index/vector/ivf/v2/tests.rs#L7506
30. [S30] https://github.com/lancedb/lancedb/blob/8029428c7152/docs/web/indexing/vector-index.mdx
31. [S31] https://github.com/lancedb/lancedb/blob/8029428c7152/docs/web/indexing/incremental-indexing.mdx
32. [S32] https://github.com/lancedb/lancedb/blob/8029428c7152/docs/web/tables/consistency.mdx
33. [S33] https://github.com/lancedb/lancedb/blob/8029428c7152/docs/web/tables/realtime-engine.mdx
34. [S34] https://github.com/lancedb/lancedb/blob/8029428c7152/docs/web/search/realtime-read.mdx
35. [S35] https://github.com/lancedb/lancedb/blob/8029428c7152/docs/web-tests/rs/search_hybrid_search.rs
36. [S36] https://github.com/lancedb/lancedb/issues/3377
37. [S37] https://arxiv.org/abs/2504.15247
38. [S38] https://crates.io/crates/lancedb
39. [S39] https://github.com/HelixDB/helix-db/blob/b524727a283f/README.md
40. [S40] https://github.com/HelixDB/helix-db/blob/b524727a283f/docs/codebase-map.md
41. [S41] https://github.com/HelixDB/helix-db/blob/b524727a283f/crates/db/Cargo.toml
42. [S42] https://github.com/HelixDB/helix-db/blob/b524727a283f/crates/server/src/http.rs#L384-L392
43. [S43] https://github.com/HelixDB/helix-db/blob/b524727a283f/crates/db/src/transaction.rs#L31-L37
44. [S44] https://github.com/HelixDB/helix-db/commits/main/LICENSE
45. [S45] https://github.com/chroma-core/chroma/blob/1a3ac2c7c623/docs/mintlify/reference/architecture/overview.mdx
46. [S46] https://github.com/chroma-core/chroma/blob/1a3ac2c7c623/docs/mintlify/guides/performance/single-node.mdx
47. [S47] https://github.com/chroma-core/chroma/blob/1a3ac2c7c623/docs/mintlify/cloud/search-api/overview.mdx
48. [S48] https://github.com/chroma-core/chroma/blob/1a3ac2c7c623/docs/mintlify/docs/querying-collections/full-text-search.mdx
49. [S49] https://pypi.org/project/chromadb/
50. [S50] https://github.com/chroma-core/chroma/tree/1a3ac2c7c623/rust
51. [S51] https://github.com/surrealdb/surrealdb/blob/9577ac5f0caa/LICENSE
52. [S52] https://github.com/surrealdb/surrealdb/blob/9577ac5f0caa/surrealdb/kvs-surrealkv/src/cnf.rs
53. [S53] https://github.com/surrealdb/surrealdb/blob/9577ac5f0caa/surrealdb/kvs-any/Cargo.toml
54. [S54] https://surrealdb.com/docs/surrealql/statements/define/indexes
55. [S55] https://surrealdb.com/releases
56. [S56] https://github.com/surrealdb/surrealkv/blob/889ae54eee7b/README.md
57. [S57] https://github.com/meilisearch/meilisearch/blob/4ef8a91f27ee/LICENSE
58. [S58] https://github.com/meilisearch/meilisearch/blob/4ef8a91f27ee/crates/milli/Cargo.toml
59. [S59] https://github.com/meilisearch/meilisearch/releases/tag/v1.22.0
60. [S60] https://github.com/nnethercott/hannoy
61. [S61] https://www.meilisearch.com/docs/learn/async/asynchronous_operations
62. [S62] https://www.meilisearch.com/docs/learn/ai_powered_search/getting_started_with_ai_search
63. [S63] https://www.sqlite.org/wal.html
64. [S64] https://www.sqlite.org/pragma.html
65. [S65] https://www.sqlite.org/fts5.html
66. [S66] https://www.sqlite.org/src/doc/begin-concurrent/doc/begin_concurrent.md
67. [S67] https://www.sqlite.org/changes.html
68. [S68] https://github.com/rusqlite/rusqlite/blob/master/libsqlite3-sys/sqlite3/sqlite3.h
69. [S69] https://github.com/asg017/sqlite-vec/releases
70. [S70] https://alexgarcia.xyz/blog/2024/sqlite-vec-stable-release/index.html
71. [S71] https://github.com/tursodatabase/libsql
72. [S72] https://docs.turso.tech/features/ai-and-embeddings
73. [S73] https://github.com/tursodatabase/turso/blob/72bef753971a/README.md
74. [S74] https://github.com/tursodatabase/turso/blob/72bef753971a/docs/manual.md
75. [S75] https://github.com/tursodatabase/turso/issues?q=is%3Aissue+is%3Aopen+corrupt
76. [S76] https://duckdb.org/docs/2.0/connect/concurrency.html
77. [S77] https://duckdb.org/docs/2.0/core_extensions/vss.html
78. [S78] https://github.com/duckdb/duckdb-vss/blob/bc1ec8b67ab1/README.md
79. [S79] https://duckdb.org/docs/2.0/core_extensions/full_text_search.html
80. [S80] https://github.com/duckdb/duckdb/releases/tag/v2.0.0
81. [S81] https://github.com/quickwit-oss/tantivy/blob/2ad36b4fd0c00ded047b12295423250d8d4d713b/ARCHITECTURE.md
82. [S82] https://github.com/quickwit-oss/tantivy/blob/2ad36b4fd0c00ded047b12295423250d8d4d713b/src/directory/directory_lock.rs#L42-L58
83. [S83] https://github.com/quickwit-oss/tantivy/blob/2ad36b4fd0c00ded047b12295423250d8d4d713b/src/indexer/index_writer.rs#L27-L36
84. [S84] https://github.com/quickwit-oss/tantivy/blob/2ad36b4fd0c00ded047b12295423250d8d4d713b/CHANGELOG.md
85. [S85] https://github.com/unum-cloud/USearch/blob/840374af1a1d/README.md
86. [S86] https://docs.rs/usearch/2.26.6/usearch/struct.Index.html
87. [S87] https://github.com/jean-pierreBoth/hnswlib-rs/blob/3669dc6ecee5/README.md
88. [S88] https://github.com/djc/instant-distance
89. [S89] https://github.com/microsoft/DiskANN/blob/13bfb8e3911b/README.md
90. [S90] https://arxiv.org/abs/2410.14452
91. [S91] https://github.com/SPFresh/SPFresh
92. [S92] https://github.com/chroma-core/chroma/tree/1a3ac2c7c623/rust/index/src/spann
93. [S93] https://github.com/cberner/redb/blob/56be42b31c28/README.md
94. [S94] https://github.com/cberner/redb/blob/56be42b31c28/CHANGELOG.md
95. [S95] https://github.com/fjall-rs/fjall/blob/87006048139b/README.md
96. [S96] http://www.lmdb.tech/doc/
97. [S97] https://github.com/meilisearch/heed
98. [S98] https://github.com/facebook/rocksdb/wiki/Basic-Operations
99. [S99] https://github.com/facebook/rocksdb/wiki/Read-only-and-Secondary-instances
100. [S100] https://github.com/spacejam/sled
101. [S101] https://github.com/slatedb/slatedb/blob/9aedb40017e4/README.md
102. [S102] https://github.com/cozodb/cozo
103. [S103] https://github.com/kuzudb/kuzu
104. [S104] https://github.com/LadybugDB/ladybug
105. [S105] https://github.com/Kineviz/bighorn
106. [S106] https://github.com/predictable-labs/ryugraph
107. [S107] https://github.com/typedb/typedb
108. [S108] https://github.com/oxigraph/oxigraph
109. [S109] https://github.com/indradb/indradb
110. [S110] https://github.com/getzep/graphiti
111. [S111] https://github.com/FalkorDB/FalkorDB/blob/master/LICENSE
112. [S112] https://github.com/FalkorDB/falkordblite
113. [S113] https://huggingface.co/blog/embedding-quantization
114. [S114] https://github.com/erikbern/ann-benchmarks
115. [S115] https://github.com/vector-index-bench/vibe
116. [S116] https://github.com/harsha-simhadri/big-ann-benchmarks (referenced via VIBE README; repo not opened directly)
117. [S117] https://github.com/rust-lang/rust/blob/master/library/std/src/sys/fs/unix.rs
118. [S118] https://www.apple.com/macbook-pro/specs/
119. [S119] https://github.com/dsrhaslab/lazyfs
120. [S120] https://hub.docker.com/r/qdrant/qdrant/tags (also getmeili/meilisearch, chromadb/chroma, surrealdb/surrealdb, falkordb/falkordb via Docker Hub API)
121. [S121] https://crates.io/ (crate versions and downloads via crates.io API)
122. [S122] https://github.com/pgvector/pgvector and https://github.com/paradedb/paradedb
123. [S123] https://github.com/qdrant/qdrant/blob/222b2e44ab1c3f17e8246396ead6558e45041df7/docs/redoc/master/openapi.json

[S1]: https://github.com/qdrant/qdrant
[S2]: https://github.com/qdrant/qdrant/releases/tag/v1.20.0
[S3]: https://github.com/qdrant/qdrant/releases/tag/v1.19.0
[S4]: https://github.com/qdrant/qdrant/releases/tag/v1.17.0
[S5]: https://qdrant.tech/documentation/edge/
[S6]: https://qdrant.tech/documentation/edge/edge-vs-qdrant-cluster/
[S7]: https://github.com/qdrant/qdrant/blob/222b2e44ab1c3f17e8246396ead6558e45041df7/lib/shard/src/wal.rs#L101-L106
[S8]: https://github.com/qdrant/qdrant/blob/222b2e44ab1c3f17e8246396ead6558e45041df7/lib/collection/src/update_workers/update_worker.rs#L358-L364
[S9]: https://github.com/qdrant/qdrant/blob/222b2e44ab1c3f17e8246396ead6558e45041df7/config/config.yaml#L190-L242
[S10]: https://github.com/qdrant/qdrant/blob/222b2e44ab1c3f17e8246396ead6558e45041df7/lib/wal/src/segment.rs#L421-L447
[S11]: https://qdrant.tech/documentation/manage-data/storage/
[S12]: https://qdrant.tech/documentation/scaling/consistency-guarantees/
[S13]: https://qdrant.tech/documentation/operations/distributed_deployment/
[S14]: https://qdrant.tech/documentation/search/hybrid-queries/
[S15]: https://qdrant.tech/documentation/manage-data/indexing/
[S16]: https://qdrant.tech/documentation/manage-data/multitenancy/
[S17]: https://github.com/qdrant/qdrant/blob/222b2e44ab1c3f17e8246396ead6558e45041df7/lib/edge/ffi/src/config.rs#L1016-L1021
[S18]: https://github.com/qdrant/qdrant/blob/222b2e44ab1c3f17e8246396ead6558e45041df7/lib/edge/src/read_only/mod.rs#L1-L13
[S19]: https://crates.io/crates/qdrant-edge
[S20]: https://github.com/qdrant/qdrant/issues/11385
[S21]: https://jepsen.io/analyses
[S22]: https://github.com/lance-format/lance/blob/8f07c195f357/docs/src/format/table/transaction.md
[S23]: https://github.com/lance-format/lance/blob/8f07c195f357/docs/src/format/table/mem_wal.md
[S24]: https://github.com/lance-format/lance/blob/8f07c195f357/docs/src/guide/object_store.md
[S25]: https://github.com/lance-format/lance/issues/9608
[S26]: https://github.com/lance-format/lance/pull/9609
[S27]: https://github.com/lance-format/lance/blob/8f07c195f357/docs/src/guide/full_text_search.md
[S28]: https://github.com/lance-format/lance/blob/8f07c195f357/docs/src/format/index/index.md
[S29]: https://github.com/lance-format/lance/blob/8f07c195f357/rust/lance/src/index/vector/ivf/v2/tests.rs#L7506
[S30]: https://github.com/lancedb/lancedb/blob/8029428c7152/docs/web/indexing/vector-index.mdx
[S31]: https://github.com/lancedb/lancedb/blob/8029428c7152/docs/web/indexing/incremental-indexing.mdx
[S32]: https://github.com/lancedb/lancedb/blob/8029428c7152/docs/web/tables/consistency.mdx
[S33]: https://github.com/lancedb/lancedb/blob/8029428c7152/docs/web/tables/realtime-engine.mdx
[S34]: https://github.com/lancedb/lancedb/blob/8029428c7152/docs/web/search/realtime-read.mdx
[S35]: https://github.com/lancedb/lancedb/blob/8029428c7152/docs/web-tests/rs/search_hybrid_search.rs
[S36]: https://github.com/lancedb/lancedb/issues/3377
[S37]: https://arxiv.org/abs/2504.15247
[S38]: https://crates.io/crates/lancedb
[S39]: https://github.com/HelixDB/helix-db/blob/b524727a283f/README.md
[S40]: https://github.com/HelixDB/helix-db/blob/b524727a283f/docs/codebase-map.md
[S41]: https://github.com/HelixDB/helix-db/blob/b524727a283f/crates/db/Cargo.toml
[S42]: https://github.com/HelixDB/helix-db/blob/b524727a283f/crates/server/src/http.rs#L384-L392
[S43]: https://github.com/HelixDB/helix-db/blob/b524727a283f/crates/db/src/transaction.rs#L31-L37
[S44]: https://github.com/HelixDB/helix-db/commits/main/LICENSE
[S45]: https://github.com/chroma-core/chroma/blob/1a3ac2c7c623/docs/mintlify/reference/architecture/overview.mdx
[S46]: https://github.com/chroma-core/chroma/blob/1a3ac2c7c623/docs/mintlify/guides/performance/single-node.mdx
[S47]: https://github.com/chroma-core/chroma/blob/1a3ac2c7c623/docs/mintlify/cloud/search-api/overview.mdx
[S48]: https://github.com/chroma-core/chroma/blob/1a3ac2c7c623/docs/mintlify/docs/querying-collections/full-text-search.mdx
[S49]: https://pypi.org/project/chromadb/
[S50]: https://github.com/chroma-core/chroma/tree/1a3ac2c7c623/rust
[S51]: https://github.com/surrealdb/surrealdb/blob/9577ac5f0caa/LICENSE
[S52]: https://github.com/surrealdb/surrealdb/blob/9577ac5f0caa/surrealdb/kvs-surrealkv/src/cnf.rs
[S53]: https://github.com/surrealdb/surrealdb/blob/9577ac5f0caa/surrealdb/kvs-any/Cargo.toml
[S54]: https://surrealdb.com/docs/surrealql/statements/define/indexes
[S55]: https://surrealdb.com/releases
[S56]: https://github.com/surrealdb/surrealkv/blob/889ae54eee7b/README.md
[S57]: https://github.com/meilisearch/meilisearch/blob/4ef8a91f27ee/LICENSE
[S58]: https://github.com/meilisearch/meilisearch/blob/4ef8a91f27ee/crates/milli/Cargo.toml
[S59]: https://github.com/meilisearch/meilisearch/releases/tag/v1.22.0
[S60]: https://github.com/nnethercott/hannoy
[S61]: https://www.meilisearch.com/docs/learn/async/asynchronous_operations
[S62]: https://www.meilisearch.com/docs/learn/ai_powered_search/getting_started_with_ai_search
[S63]: https://www.sqlite.org/wal.html
[S64]: https://www.sqlite.org/pragma.html
[S65]: https://www.sqlite.org/fts5.html
[S66]: https://www.sqlite.org/src/doc/begin-concurrent/doc/begin_concurrent.md
[S67]: https://www.sqlite.org/changes.html
[S68]: https://github.com/rusqlite/rusqlite/blob/master/libsqlite3-sys/sqlite3/sqlite3.h
[S69]: https://github.com/asg017/sqlite-vec/releases
[S70]: https://alexgarcia.xyz/blog/2024/sqlite-vec-stable-release/index.html
[S71]: https://github.com/tursodatabase/libsql
[S72]: https://docs.turso.tech/features/ai-and-embeddings
[S73]: https://github.com/tursodatabase/turso/blob/72bef753971a/README.md
[S74]: https://github.com/tursodatabase/turso/blob/72bef753971a/docs/manual.md
[S75]: https://github.com/tursodatabase/turso/issues?q=is%3Aissue+is%3Aopen+corrupt
[S76]: https://duckdb.org/docs/2.0/connect/concurrency.html
[S77]: https://duckdb.org/docs/2.0/core_extensions/vss.html
[S78]: https://github.com/duckdb/duckdb-vss/blob/bc1ec8b67ab1/README.md
[S79]: https://duckdb.org/docs/2.0/core_extensions/full_text_search.html
[S80]: https://github.com/duckdb/duckdb/releases/tag/v2.0.0
[S81]: https://github.com/quickwit-oss/tantivy/blob/2ad36b4fd0c00ded047b12295423250d8d4d713b/ARCHITECTURE.md
[S82]: https://github.com/quickwit-oss/tantivy/blob/2ad36b4fd0c00ded047b12295423250d8d4d713b/src/directory/directory_lock.rs#L42-L58
[S83]: https://github.com/quickwit-oss/tantivy/blob/2ad36b4fd0c00ded047b12295423250d8d4d713b/src/indexer/index_writer.rs#L27-L36
[S84]: https://github.com/quickwit-oss/tantivy/blob/2ad36b4fd0c00ded047b12295423250d8d4d713b/CHANGELOG.md
[S85]: https://github.com/unum-cloud/USearch/blob/840374af1a1d/README.md
[S86]: https://docs.rs/usearch/2.26.6/usearch/struct.Index.html
[S87]: https://github.com/jean-pierreBoth/hnswlib-rs/blob/3669dc6ecee5/README.md
[S88]: https://github.com/djc/instant-distance
[S89]: https://github.com/microsoft/DiskANN/blob/13bfb8e3911b/README.md
[S90]: https://arxiv.org/abs/2410.14452
[S91]: https://github.com/SPFresh/SPFresh
[S92]: https://github.com/chroma-core/chroma/tree/1a3ac2c7c623/rust/index/src/spann
[S93]: https://github.com/cberner/redb/blob/56be42b31c28/README.md
[S94]: https://github.com/cberner/redb/blob/56be42b31c28/CHANGELOG.md
[S95]: https://github.com/fjall-rs/fjall/blob/87006048139b/README.md
[S96]: http://www.lmdb.tech/doc/
[S97]: https://github.com/meilisearch/heed
[S98]: https://github.com/facebook/rocksdb/wiki/Basic-Operations
[S99]: https://github.com/facebook/rocksdb/wiki/Read-only-and-Secondary-instances
[S100]: https://github.com/spacejam/sled
[S101]: https://github.com/slatedb/slatedb/blob/9aedb40017e4/README.md
[S102]: https://github.com/cozodb/cozo
[S103]: https://github.com/kuzudb/kuzu
[S104]: https://github.com/LadybugDB/ladybug
[S105]: https://github.com/Kineviz/bighorn
[S106]: https://github.com/predictable-labs/ryugraph
[S107]: https://github.com/typedb/typedb
[S108]: https://github.com/oxigraph/oxigraph
[S109]: https://github.com/indradb/indradb
[S110]: https://github.com/getzep/graphiti
[S111]: https://github.com/FalkorDB/FalkorDB/blob/master/LICENSE
[S112]: https://github.com/FalkorDB/falkordblite
[S113]: https://huggingface.co/blog/embedding-quantization
[S114]: https://github.com/erikbern/ann-benchmarks
[S115]: https://github.com/vector-index-bench/vibe
[S116]: https://github.com/harsha-simhadri/big-ann-benchmarks
[S117]: https://github.com/rust-lang/rust/blob/master/library/std/src/sys/fs/unix.rs
[S118]: https://www.apple.com/macbook-pro/specs/
[S119]: https://github.com/dsrhaslab/lazyfs
[S120]: https://hub.docker.com/r/qdrant/qdrant/tags
[S121]: https://crates.io/
[S122]: https://github.com/pgvector/pgvector
[S123]: https://github.com/qdrant/qdrant/blob/222b2e44ab1c3f17e8246396ead6558e45041df7/docs/redoc/master/openapi.json
