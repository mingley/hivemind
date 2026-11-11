# Research 06: Architecture precedents (concurrency, durability, sync, distribution, protocols, security)

> Research thread report, checked 2026-11-10. Produced by a research agent from public sources and saved verbatim. Evidence labels: **[V]** verified in a primary source, **[C]** project or vendor claim, **[I]** independent evidence, **[Inf]** inference, **[?]** unknown. Synthesis and decisions live in [`docs/`](../docs/).

## Executive summary

- **Shape:** make an append-only event log, partitioned by tenant and project, the source of truth. Search, vector, graph and current-state views are projections that can be rebuilt from it. Datomic, XTDB, turbopuffer, distributed Chroma and Milvus 3 all work this way. Record system time on every event and keep valid time optional. A correction is a new event that supersedes the old one. Erasure is a separate, rare, audited operation; Datomic's docs say excision "should never be used to correct erroneous data".
- **Many agents on one laptop:** coding agents start MCP stdio servers as subprocesses, so several processes end up writing one store. The official MCP reference memory server lost writes this way twice: #4555 within one process and #4797 across processes. It was fixed with a file lock in October 2026. For v0, one daemon should own the store, auto-started and exiting when idle as sccache does, behind a thin stdio shim. MCP's stdio framing works unchanged over a Unix domain socket.
- **Storage:** use SQLite in WAL mode for v0. Its limits are documented: one writer at a time, same host only, no network filesystems. redb's multi-process modes shipped only on 2026-11-05. fjall can't be opened by more than one process. Turso is still 0.8.x, with multi-process support, full-text search and encryption marked experimental. Don't build a custom engine.
- **Sync:** memory events never change, so replication is a set union keyed by ID or hash. Status flags can use last-writer-wins or multi-value registers. Leases can't be merged this way; they need one authoritative server and fencing tokens. Use Litestream 0.5.x for backup and point-in-time recovery; LiteFS is still beta and cr-sqlite has had no release since January 2024. PowerSync's upload-queue-and-checkpoint pattern fits syncing a laptop with a team service.
- **Distribution (v2 only, when the triggers fire):** the precedents keep immutable data in object storage and advance one pointer with compare-and-swap (Datomic, turbopuffer, Litestream, Lance). Use Raft (openraft 0.10, API still pre-1.0) only for highly available metadata and leases. Vector databases use Raft for metadata but weaker data replication. Qdrant's own docs say a tenant filter is not access control.
- **Protocols:**
  - **MCP 2026-07-28** (stable) is stateless: no sessions or initialize handshake. It adds `server/discover` and `subscriptions/listen`, and drops stream resumability. Tasks moved to a Beta extension. OAuth 2.1 now prefers Client ID Metadata Documents; Dynamic Client Registration is deprecated.
  - **rmcp** (Tier 1, v3.9.0) implements it.
  - **Notifications:** don't rely on push. Provide `changes_since(cursor)`.
  - **Other protocols:** tonic now lives at grpc/grpc-rust, and A2A reached v1.0.
  - **Telemetry:** the OpenTelemetry GenAI conventions, now in their own repo with Development status, already name memory operations and token-usage attributes. Adopt those names.
- **Security:** memory poisoning has been demonstrated against ChatGPT, Gemini, Windsurf and Bedrock Agents. In published research, AgentPoison succeeded over 80% of the time with under 0.1% of memory poisoned, and PoisonedRAG reached 90% with five injected texts. v0 needs:
  - trust tiers based on where content came from, with untrusted-sourced writes quarantined by default
  - writes attributed to their author
  - scope ACLs enforced on the server before ranking
  - secret redaction when content is ingested
  - a hash-chained audit log
  - an erase pipeline that also purges projections
- **Build into v0 so later stages aren't blocked:** UUIDv7 IDs plus content hashes, per-partition log offsets used as cursors, idempotency keys, versioned canonical events, payloads stored separately from envelopes, lease fencing tokens, and reserved signature and hybrid-logical-clock fields.

---

## Hivemind research 06: architecture precedents for concurrency, durability, sync, distribution, protocols and security

*Checked 2026-11-10. Public sources only.*
**Legend:** [V] verified in a primary source · [C] vendor/project claim · [I] independent evidence · [Inf] inference · [?] unknown. Bracketed numbers refer to §11 Sources.

## 1. Summary

1. **Use an append-only event log as the source of truth.** Search, vector, graph and current-state views are projections you can rebuild. This is the shared core of Datomic, XTDB v2, turbopuffer, distributed Chroma and Milvus 3 [27][28][86][94][93]. [V/C]
2. **System time is required; valid time is an optional field the writer asserts.** Don't build a bitemporal query engine. A correction is a new event that supersedes or retracts the old one. Erasure is separate and rare: Datomic says excision "should never be used to correct erroneous data" [25], and XTDB `ERASE` exists for GDPR [29]. [V]
3. **One local store will have many processes.** MCP stdio means the client launches the server as a subprocess [4], so each agent session brings its own process. The official reference memory server had a lost-update race inside one process (#4555, merged 2026-07-27). It then had silent lost writes across two processes sharing a file (#4797, fixed with a cross-process lock on 2026-10-04) [58]. [V]
4. **Local daemon pattern.** One `hivemindd` process owns the store. `hivemind mcp` is a thin stdio shim that starts the daemon if needed and exits when idle, the way sccache does [55]. It talks to the daemon over a Unix domain socket; MCP stdio framing "works unchanged over Unix domain sockets" [4]. [V+Inf]
5. **Engine for v0 is SQLite in WAL mode.** It allows one writer at a time, works only on one host, and doesn't work on network filesystems [43]. It has FTS5 and works with Litestream. The alternatives:
   - redb added multi-process modes only on 2026-11-05 [50].
   - fjall can't be opened by more than one process [51].
   - DuckDB allows one read-write process [53].
   - Turso is 0.8.3, with multi-process, FTS and encryption experimental [54].

   [V]
6. **Backup and disaster recovery: Litestream 0.5.x.** It adds the LTX format, compaction, point-in-time recovery, a single replicator per destination via conditional writes, and VFS read replicas [61][62][63]. LiteFS is still beta [64]. [V/C]
7. **Sync is a set union, not a general CRDT problem.** Immutable events behave as a G-Set (grow-only set). Mutable flags can be last-writer-wins (LWW) or multi-value (MV) registers. Leases need a linearizable authority plus fencing tokens [99]. [Inf]
8. **Each operation needs a different guarantee.** Agents need read-your-writes for their own memories and causal consistency for handoffs. Shared knowledge can be eventually consistent with bounded staleness. Leases need linearizability, and deletes must propagate strongly to every projection. One log offset ("commit token") returned on write and accepted on read gives both read-your-writes and causal reads on a single node [97][98]. [Inf]
9. **Distributed precedents agree on immutable data in object storage plus one compare-and-swap (CAS) pointer.**
   - Datomic: a single transactor's CAS caps write throughput [27].
   - turbopuffer: one WAL CAS per second per namespace; p50 write latency 165 ms [86].
   - Litestream: "CASAAS" (compare-and-swap as a service) [61].
   - Lance: optimistic concurrency control (OCC) [42].

   Raft (openraft 0.10, API pre-1.0 [82]) belongs to HA metadata and leases. [I/C]
10. **Vector databases replicate data weakly.** Qdrant's default `write_consistency_factor` is 1, and concurrent updates to one point can diverge across replicas [89]. Weaviate replicates data leaderlessly with tunable consistency [91]. Qdrant's docs also state that its tenant index "is not access control" [90]. [V]
11. **MCP 2026-07-28 changes the protocol substantially.** It is stateless, adds `server/discover` and `subscriptions/listen`, removes SSE resumability, and moves Tasks into an extension. Multi Round-Trip Requests (MRTR) replace server-initiated requests. List results carry `ttlMs` cache hints, and servers should return tools in a deterministic order. Authorization is OAuth 2.1 with Protected Resource Metadata (PRM) and Client ID Metadata Documents (CIMD), and Dynamic Client Registration (DCR) is deprecated [1][6]. rmcp is Tier 1 and implements it as of v3.9.0 [11][12]. [V]
12. **Don't depend on push.** Support for subscriptions and Tasks across coding-agent clients is uneven or unknown [9]. Ship cursor polling (`changes_since`) and server-minted handles. [V/?]
13. **Other protocols.** tonic 0.14.9 now lives at grpc/grpc-rust, and its master branch is preparing breaking changes [13]. A2A v1.0 was released 2026-03-12 [14]. [V]
14. **Telemetry.** The OpenTelemetry GenAI semantic conventions moved to their own repo and are in Development status. They define the `create/search/update/upsert/delete_memory` operations, `gen_ai.memory.*` attributes, token usage including cache read/write, and MCP spans and metrics [16][17][18][19]. [V]
15. **Memory poisoning has been demonstrated** [102]–[108]:
    - ChatGPT "SpAIware" (2024)
    - Gemini (2025)
    - Windsurf's automatically invoked `create_memory` (2025)
    - Bedrock Agents (Unit 42, 2025)
    - AgentPoison: over 80% attack success at under 0.1% poison rate
    - PoisonedRAG: 90% with five texts per target

    Trust tiers, quarantine, server-side ACLs, redaction, audit and an erase pipeline are v0 requirements. [I]

---

## 2. Immutable and bitemporal fact stores

### 2.1 Datomic

- **Model:**
  - **Datoms:** `[entity, attribute, value, tx, added?]`. Every transaction has a strictly monotonic `t` and a wall-clock `txInstant`; transactions assert or retract datoms, and "the database is an ever-growing set of these tuples" [27]. [I: Jepsen background]
  - **Time travel:** `as-of`, `since` and `history` filters let one set of queries run against different time views [24]. [V]
- **Architecture:**
  - Transactors execute writes, maintain indexes and write to storage; peers are thick clients [27]. [I]
  - A commit writes immutable tree nodes to eventually consistent storage, then uses a sequentially consistent CAS to advance the root pointer. Jepsen notes this "limits Datomic's write throughput to the speed of a single transactor" [27]. [I]
- **Demonstrated (Jepsen, 2024-05-15, Pro 1.0.7075):**
  - Safety between transactions was stronger than claimed: Serializable; Strong Session Serializable per peer; Strong Serializable with `d/sync`.
  - Inside a transaction, though, operations behave as if evaluated concurrently. That creates "the potential for invariant violations when composing transaction functions" [27]. [I]
- **Erasure:** excision removes datoms completely, for privacy or retention. It "should never be used to correct erroneous data… Consider using ordinary retraction" [25]. [V]
- **License:** all editions became free on 2023-04-27, with binaries under Apache 2.0. That covers the binaries, not an open-source codebase [23]. [V]
- **Lessons for hivemind** [Inf]:
  - A single writer plus immutable storage plus a CAS root pointer is a proven way to scale reads.
  - Specify what happens inside a batch: does a `supersede` see an earlier `claim` in the same batch?
  - Correction and erasure must be separate operations.

### 2.2 XTDB v2

- **Claims [C] [28]:** an immutable SQL database with time travel. It is bitemporal: system time is automatic and valid time is user-editable. It implements SQL:2011 bitemporal semantics over the Postgres wire protocol, on an Apache Arrow columnar engine built for object storage. Its "inside-out" architecture uses the log as the coordination point. It is "optimised for current-time queries."
- **Implemented [V]:**
  - Rows have valid-from/to and system-from/to columns; valid time is "another, user-editable timeline" [30].
  - `DELETE` defaults to now→end-of-time, which the docs flag as a deviation from SQL:2011.
  - `ERASE` "irrevocably erases documents… for all valid-time, for all system-time… e.g. for a GDPR request" [29].
  - `ASSERT` gives transaction-level constraint checks [29].
- **Maturity [V] (gh):** v2.2.0 on 2026-10-21, MPL-2.0, active.
- **Lesson [Inf]:** an immutable store still needs a hard-delete path. Even XTDB, a vendor built around immutability, added one.

### 2.3 SQL:2011 temporal tables

- **Standard:** SQL:2011 defines application-time period tables, system-versioned tables, and bitemporal tables that combine both [31]. [V]
- **MariaDB:** implements system versioning with `FOR SYSTEM_TIME AS OF/BETWEEN/ALL`, application-time periods, and bitemporal tables [33]. [V]
- **PostgreSQL 18:** adds `WITHOUT OVERLAPS` for primary-key and unique constraints and `PERIOD` for foreign keys [32]. [V] I did not find system-versioned tables in the PG18 release notes. [?]
- **Lesson [Inf]:** if hivemind ever exposes SQL or a time filter, reuse the SQL:2011 terms (`valid_from`/`valid_to`, `AS OF`).

### 2.4 TerminusDB

- **What it is [V] [34]:** "git-for-data" for JSON, JSON-LD and RDF documents with a schema, supporting push and pull. v12 is "maintained by the Gettier team".
- **Maturity [V]:** Apache-2.0; v12.1.0-rc2 on 2026-11-03.
- **Lesson [Inf]:** a commit graph suits curated, low-rate knowledge such as team playbooks. It is heavy for high-rate streams of observations.

### 2.5 Dolt

- **What it is [V] [35][36]:** a MySQL-compatible SQL database you can fork, clone, branch, merge and push. Doltgres, the Postgres-compatible variant, is at v1.5.0 (2026-11-05). Merges are cell by cell, and conflicts arise when both sides change the same cell.
- **Maturity [V]:** Dolt v2.7.3 on 2026-11-11, Apache-2.0.
- **Lesson [Inf]:** branches map neatly onto speculative memory for an agent or task that a reviewer later merges. If writes are append-only, though, merging is just a union, so v0 doesn't need Dolt's machinery.

### 2.6 What hivemind should take from these [Inf]

| Concept | Hivemind form |
|---|---|
| Transaction/system time | `recorded_at` plus a per-partition log offset, both assigned by the log and immutable |
| Valid time | Optional `valid_from`/`valid_to` asserted by the writer (e.g. "deprecated as of v3.2"). Used in filters, not in bitemporal joins |
| Correction | A `supersede(old_id → new event)` edge. Default retrieval returns current heads; history stays available |
| Retraction | `retract(id, reason)` with no replacement |
| Erasure | `erase(selector)`: privileged, audited and rare. Purges the log payload, every projection, and eventually backups |
| As-of | "What did hivemind know at offset N?", applied to the log and metadata projection only. Mainly for reproducing what an agent saw |

**Cost of bitemporality [Inf]:** two time axes multiply index and query complexity, and most agent queries only want the current belief. Reconstructing an ANN index as of a past time is expensive, so limit as-of queries to the log and metadata.

---

## 3. Event sourcing and CQRS for memory

- **Pattern:** an event log enables "Complete Rebuild", "Temporal Query" and "Event Replay" [37]. [V]
- **Log first, then asynchronous indexing, in production search systems:**
  - **turbopuffer [C] [86]:** each namespace has a WAL on object storage. Data is indexed asynchronously after commit, and "data that has not yet been indexed is still available to search… with a slower exhaustive search." It is strongly consistent by default.
  - **Distributed Chroma [C] [94]:** the gateway turns writes into "a log of operations"; a compactor builds vector, full-text and metadata indexes from the log; the query executor "consults the log to serve a consistent result."
  - **Milvus 3.0.0 [C] [93]:** released 2026-07-29. Its Woodpecker WAL can run as a standalone service. Online column add and backfill make an "embedding-model upgrade across hundreds of millions of rows… a hot path with no downtime."
  - **Implication [Inf]:** hivemind should search the unindexed tail exhaustively. Projections should carry a version tied to the embedding model, and re-embedding is a backfill.
- **Idempotency:**
  - Stripe uses idempotency keys so that retries don't create a second object [38]. [V]
  - LanceDB uses optimistic concurrency, and its docs warn that "retrying an append can therefore insert the same records again" [42]. [V]
  - **Implication [Inf]:** every write takes a client idempotency key. The server dedupes on it and also on the content hash.
- **Optimistic concurrency:** KurrentDB lets an append specify the expected state of the stream [41]. [V] Hivemind's equivalent is "supersede only if the head is still X". [Inf]
  - KurrentDB is licensed under Kurrent License v1, not OSI-approved [41]. [V]
- **Schema evolution [Inf]:** version each event type, upcast old versions on read, and never rewrite history except through erasure. Each projection declares which event versions it understands.
- **Snapshots and compaction [Inf]:** projections checkpoint the offset they have reached, and the current-state projection can be snapshotted. Compact the log only for erased payloads and for retention-expired, low-value events. Litestream's LTX compaction is the storage-level analogue [61]. [C]
- **Deletion in an append-only system:**
  - **Forgettable payloads:** keep sensitive payloads in a separate store you can delete from [39]. [V]
  - **Crypto-shredding:** encrypt with a per-resource key and delete the key. It is "only as good as your encryption and your key management" [40]. [V]
  - **Derived copies [Inf]:** embeddings, summaries, FTS indexes, caches and backups must be purged too. Treat embeddings of sensitive text as sensitive.
  - **Legal standing [?]:** whether crypto-shredding counts as erasure under GDPR was not verified here and likely depends on jurisdiction.

**Proposed event envelope [Inf]:**

| Field | Purpose |
|---|---|
| `event_id` | UUIDv7 [100] |
| `partition` | (tenant, project) |
| `origin_id`, `origin_seq` | Stable per installation and monotonic, for dedupe in later replication |
| `offset` | Assigned by the log |
| `idempotency_key` | Client-supplied retry key |
| `type`, `type_version` | Event type and schema version |
| `content_hash` | BLAKE3 of the canonical payload |
| `payload_ref` | Separate blob store, so payloads can be forgotten or shredded |
| `actor` | principal, agent_id, client, session_id |
| `scope` | private / project / team / shared |
| `provenance` | source kind, URI or path plus commit SHA, `derived_from[]` |
| `trust_tier` | Provenance-based trust level |
| `recorded_at` | System time |
| `valid_from`, `valid_to` | Optional |
| `supersedes[]`, `retracts[]` | Correction edges |
| `hlc` | Reserved [101] |
| `sig` | Reserved [118] |
| `key_id` | Reserved, for crypto-shredding |

---

## 4. Embedded concurrency: many agents on one machine

### 4.1 Concurrency and durability models (table)

| Engine | Multiple processes on one file? | Writers | Readers | Key caveats | Maturity (gh) |
|---|---|---|---|---|---|
| **SQLite WAL** | Yes, same host only [V][43] | "only one writer at a time" [V][43] | Readers and writers don't block each other [V][43] | Doesn't work on network filesystems; WAL needs shared memory [V][43]. A long reader causes checkpoint starvation [V][43]. `BEGIN IMMEDIATE` may return `SQLITE_BUSY` [V][45]. Upgrading a read to a write returns `SQLITE_BUSY_SNAPSHOT` [V][46]. `busy_timeout` sleeps and retries [V][47]. With `synchronous=NORMAL`, WAL "is always consistent" but a commit "might roll back following a power loss" [V][119]. SQLite itself recommends client/server or a same-host proxy for network access [V][44] | Mature |
| SQLite `BEGIN CONCURRENT` | Same rules | Several transactions run with optimistic page-level locking; "still serializes COMMIT" [V][48] | — | Documented on the `begin-concurrent` branch; probably not in default release builds [Inf] | Branch |
| **LMDB** (via heed in Rust) | Yes, threads and processes [V][49] | "only one write transaction may be active at a time" [V][49] | Lock-free MVCC [V][49] | "Do not use LMDB databases on remote filesystems, even between processes on the same host"; long-lived transactions grow the file; stale readers need checks [V][49] | LMDB 1.0 docs; heed v0.23.1 (2026-11-02) [V] |
| **redb** | Since 4.5.0 (2026-11-05): `ExclusiveWriter` (default), `SingleWriter`, and `MultiWriter` ("one write transaction at a time") [V][50] | One at a time | MVCC | Locking is incompatible with the experimental multi-process mode in 4.3/4.4 [V][50]. Five days old: no field evidence yet [Inf] | Apache-2.0; "file format is stable" [V][50] |
| **fjall** | **No**: "may not be loaded in parallel from separate processes" [V][51] | Threads; optimistic or single-writer transactions [V][51] | Snapshot reads | LSM tree | 3.1.13 [V] |
| **RocksDB** | One primary, plus read-only and secondary instances [V][52] | Primary only | Secondaries catch up when called [V][52] | — | rust-rocksdb v0.25.0 [V] |
| **DuckDB** | One read-write process, or many read-only [V][53] | Threads in that one process (MVCC plus optimistic) [V][53] | — | Built for analytics | v2.0.0 (2026-10-21) [V] |
| **Turso (Rust SQLite rewrite)** | "Multi-process WAL coordination" is **experimental** [V][54] | `BEGIN CONCURRENT` with MVCC [V/C][54] | — | FTS (tantivy) and encryption experimental; vector indexing on the roadmap [V][54]. Tested with deterministic simulation and Antithesis [C][54] | v0.8.3, pre-1.0 [V] |

**Durability, demonstrated elsewhere [I]:** Jepsen found NATS JetStream 2.12.1 lost committed writes partly because it flushed to disk "every two minutes, rather than before acknowledging them" [96]. Hivemind must be explicit about when an acknowledged write is fsynced.

### 4.2 MCP stdio means many processes on one database

- **Spec:** "the client launches the MCP server as a subprocess" [4]. [V] Claude Code and Codex both configure local stdio servers as well as HTTP servers [21][22]. [V]
- **Evidence from the official reference memory server [V] [58]:**
  - #4555 (merged 2026-07-27): every mutation did load→mutate→save, so close-together tool calls "silently discard[ed] the other's changes."
  - #4797 (opened 2026-09-12): "two server processes sharing MEMORY_FILE_PATH silently discard each other's writes (the #4555 mutex is per-process)." It was fixed by a cross-process file lock around each mutation (commit `7be08e9`, 2026-10-04), followed by several Windows and stale-lock fixes.
- **Costs of letting every agent open the store directly [Inf]:**
  - N copies of the embedding model, ANN index and caches.
  - Change notifications between processes need polling.
  - Writers contend for the lock.
  - Schema migrations can race at startup.
  - Network filesystems (synced home folders) are unsafe for SQLite and LMDB.

### 4.3 Precedents for the local daemon pattern

- **sccache:** "client-server model, where the server runs locally… The sccache command will spawn a server process if one is not already running." `SCCACHE_IDLE_TIMEOUT` sets how long the daemon waits before exiting [55]. [V]
- **Ollama:** binds 127.0.0.1:11434 by default [56]. [V]
- **Docker:** runs through "a non-networked UNIX socket" by default [57]. [V]
- **Peer credentials:** `SO_PEERCRED` on Linux identifies the process at the other end of a Unix socket [59]. [V] The macOS equivalent was not verified here. [?]

### 4.4 Recommendation [Inf]

- **Default:** `hivemind mcp` is a small stdio shim. It finds or starts `hivemindd` (single-instance lock; Unix socket with mode 0600 inside a 0700 directory), checks versions on connect, and the daemon exits when idle.
- **Fallbacks:**
  - An embedded library mode for tests.
  - A direct-file mode for sandboxes that forbid daemons: SQLite WAL with `BEGIN IMMEDIATE`, a busy timeout, and a refusal to run on network filesystems.
- **Measure before committing:** shim cold-start time, RAM per agent, and write p99. If the daemon holds no in-memory model or index, direct-file mode might be enough.

---

## 5. Replication and sync

### 5.1 Options (table)

| Option | What it buys | Costs | Maturity / evidence | When it's justified for hivemind |
|---|---|---|---|---|
| **Litestream 0.5.x** | Continuous backup to S3 or files, point-in-time recovery via LTX compaction, a single replicator per destination via conditional writes ("CASAAS") [C][61][62]. VFS read replicas and `PRAGMA litestream_time` (2025-12-11) [C][63] | A sidecar process; async replication means a small RPO | v0.5.18 (2026-11-03), Apache-2.0; Windows not officially supported [V][60] | **v0/v1 backup and disaster recovery.** Adopt when losing the laptop or server store becomes unacceptable |
| LiteFS | FUSE-based replication with failover | FUSE, Consul leasing [C][61] | "currently in a beta state"; last release 2025-04-22 [V][64]. Fly moved its focus back to Litestream [C][62] | Avoid |
| Turso embedded replicas / sync | A local replica that reads its own writes; others see writes after `sync()` [C][65]. Sync is "last push wins"; a pull rolls back local changes, applies remote ones and replays local ones [C][66] | Tied to Turso Cloud; docs warn not to open the database during sync ("data corruption") [V][65] | Database v0.8.3 [V]. AgentFS (an agent filesystem built on Turso) is adjacent prior art [C][67] | Watch it. Re-evaluate at Turso 1.0 |
| rqlite | Raft-replicated SQLite as a single binary; CDC; read consistency `none/weak/linearizable/strong/auto` [V][68] | Server process; every write goes through Raft | v10.7.1 (2026-11-10), MIT. "Jepsen-style testing" is project-run [C][68]. No jepsen.io analysis [V][95] | v2 HA alternative if SQL semantics are kept |
| dqlite | Raft-replicated embedded SQLite (C library) | C/FFI; "slightly modified" LGPLv3 [V][69] | v1.18.7 (2026-07-02) | Unlikely; Rust options exist |
| hiqlite | Embedded SQLite plus openraft; encrypted S3 backups; claims 24.5k inserts/s [C][84] | Young project with one main maintainer [Inf] | v0.16.0 (2026-11-04), Apache-2.0 [V] | v2 prototype candidate |
| cr-sqlite | CRDT merging for SQLite tables | — | Last release 2024-01-17; 2026 commits are build fixes only [V][70] | **Don't build on it** [Inf] |
| ElectricSQL | Read-path sync out of Postgres via HTTP "shapes" and CDN fan-out; writes go through your own API [V][71] | Requires Postgres | 1.0 (2025-03-17) [V][71] | Only if v1 adopts Postgres and a browser or IDE client needs live views |
| PowerSync | Client SQLite kept in sync with Postgres, MongoDB, MySQL or SQL Server. Designed for causal+ consistency; client changes go to an upload queue, and the client never resolves conflicts locally [C][73] | Service plus a backend integration | Apache-2.0 [V][72]. Independent Jepsen-based tests found full causal consistency, atomicity and convergence in a no-fault environment [I][74] | **Pattern to copy** for laptop↔team sync, even without the product [Inf] |
| CRDT libraries (Automerge 3, Loro 1.x, yrs) | Merging collaborative documents (text, trees) | History overhead; Automerge 3 claims a more than 10× memory cut [C][75] | Automerge js 3.5.0, Loro 1.17.1, yrs 0.28.1, all MIT [V] | Only for co-edited long-form notes. Not for the core log |
| iroh 1.x plus iroh-blobs / iroh-docs | QUIC, hole-punching with relay fallback, BLAKE3 content-addressed blobs [V][78]. iroh-docs entries signed with a **namespace key (write capability) and an author key**, synced by range-based set reconciliation [V][79] | NAT and relay infrastructure, key management; iroh-docs is still 0.x | iroh 1.0 (2026-06-15) → v1.4.1. iroh-docs v0.100.0, iroh-blobs v0.104.1 [V] | Peer-to-peer laptop↔laptop sync with no server. Good precedent for signed, attributed writes |

### 5.2 Which memory operations are CRDT-friendly [Inf]

| Operation | CRDT type | OK? |
|---|---|---|
| Append an observation or claim (immutable, content-addressed) | G-Set (union) | ✔ |
| Supersede/retract edges; computing the current head | G-Set of edges plus a deterministic head rule; surface forks | ✔ |
| Add or remove tags and labels | OR-Set (observed-remove set) | ✔ |
| Status, pinned, title | LWW register (with hybrid logical clocks, HLC [101]) or MV register that surfaces conflicts | ✔ with caveats |
| Usage counts and votes | PN-counter (positive-negative counter) | ✔ |
| Leases and claims, uniqueness, quotas | Need consensus or linearizable CAS plus fencing tokens [99] | ✘ |
| Erasure | Tombstones persist; needs out-of-band key destruction or a coordinated purge | ✘ |

Local-first ideals (offline use, user ownership) are set out in [81]. [V]

---

## 6. Consensus and distribution

### 6.1 Raft in Rust

- **openraft:**
  - v0.10.0 released 2026-11-10, Apache-2.0 [V].
  - Used for Databend's meta-service cluster [C][82].
  - Runs a deterministic simulation fuzzer and a Jepsen suite (8 nemesis scenarios) in CI [C][82].
  - "Upgrades before 1.0.0 may include incompatible changes" [V][82].
- **raft-rs (TiKV):** last release v0.7.0 on 2023-03-07 [V][83].
- **hiqlite:** see §5.1.
- **Evidence that would justify Raft [Inf]:** an availability SLO that a single node plus Litestream restore can't meet, measured including deploys and restore time. Or a need for zero-RPO leases.

### 6.2 Object-storage-native designs

- **SlateDB [C] [85]:** an LSM tree whose SSTs live in object storage. It batches writes into periodically flushed MemTables because PUTs cost money, so writes have higher latency. v0.18.1 [V].
- **turbopuffer [C] [86]:**
  - Each namespace has a WAL prefix; "if a write returns successfully, data is guaranteed to be durably written to object storage."
  - Write p50 is 165 ms for 500 kB, at one WAL entry per second per namespace with batching.
  - Cold query p50 is 874 ms versus 14 ms warm (1M documents).
  - Strong consistency by default; in eventual mode, staleness can reach "about one hour" in the worst case.
- **WarpStream [C] [87]:** a "single stateless binary (Agent)" talks only to object storage plus a cloud metadata store; any agent can lead any topic.
- **S3 conditional writes** (August 2024) make CAS commits on S3 possible [88]. [V title]
- **Common pattern [Inf]:** immutable segments plus one linearizable pointer, as in Datomic [27], turbopuffer, Litestream and Lance [42]. You get bottomless, cheap durability and stateless compute. You pay 100 ms-scale write latency, cold-cache latency and a cloud dependency.

### 6.3 How vector and search databases shard and replicate

| System | Metadata | Data replication | Tenancy | Version (gh) |
|---|---|---|---|---|
| Qdrant | Raft; three nodes minimum for a majority [V][90] | Writes go in parallel to all active replicas. `write_consistency_factor` defaults to 1. Concurrent updates to one point can leave replicas disagreeing unless the `ordering` parameter is used [V][89] | `is_tenant` payload index or custom shard keys. Docs: "The index is not access control. Take the tenant ID from the authenticated user" [V][90] | v1.20.0 |
| Weaviate | Raft for collection definitions and tenant status [V][91] | Leaderless with tunable consistency; r + w > n [V][91] | One dedicated shard per tenant [V][92] | v1.40.5 |
| Milvus 3.0 | — | Woodpecker WAL (standalone option); Storage V3 manifests on object storage [C][93] | — | v3.0.5 |
| Chroma (distributed) | SQL-backed system database [C][94] | Log plus compactor plus query executors on object storage, with SSD caches; rendezvous hashing on collection ID [C][94] | Tenant → database → collection [C][94] | 1.5.9 |

There are no jepsen.io analyses of Qdrant, Weaviate, Milvus, Chroma, rqlite, Litestream or LiteFS as of the index checked [95]. [V]

### 6.4 What consistency agents actually need [Inf]

| Operation | Guarantee | Mechanism, single node first |
|---|---|---|
| An agent reading its own memories | Read-your-writes [97] | The write returns `offset`; the read passes `min_offset` and waits until projections reach it |
| Handoff ("pick up where A left off") | Causal [98] | The handoff event records the maximum offset it depends on; the reader waits for that offset |
| Shared team knowledge | Eventual with bounded staleness | Asynchronous projections; staleness reported in results |
| Leases on work items | Linearizable CAS plus fencing tokens [99] | Daemon or service is the single authority; stale tokens rejected |
| Erase/redact | Strong, with a deadline | Purge through every projection with acknowledgement |
| Change notifications | At-least-once with cursor resume | `changes_since(cursor)`; push is an optimization |

### 6.5 Tenant-based sharding [Inf]

The natural partition is (tenant, project): each partition gets its own log and a single writer, and no transaction spans partitions. Weaviate's per-tenant shards [92] and turbopuffer's per-namespace WAL [86] are precedents. Per-partition files also make erasure, backup and moving a partition simple.

---

## 7. Protocols

### 7.1 MCP 2026-07-28 (current stable, released 2026-07-28) [V] [1][2]

- **Stateless:**
  - The `Mcp-Session-Id` header and the `initialize` handshake are gone.
  - Every request carries its protocol version and client capabilities in `_meta`.
  - Servers keep cross-call state in "explicit, server-minted handles."
  - Servers must implement `server/discover`.
- **Notifications:** `subscriptions/listen` replaces `resources/subscribe` and the GET endpoint. It is a long-lived stream filtered by `toolsListChanged`, `resourcesListChanged` and `resourceSubscriptions` [3]. "The server holds no subscription state across reconnections" (stdio) [3].
- **Removed:** SSE resumability and `Last-Event-ID`. "A broken response stream loses the in-flight request" [1].
- **Tasks:** moved to the `io.modelcontextprotocol/tasks` extension, at stage "Beta, to Core". It polls via `tasks/get`, takes input via `tasks/update`, and returns durable handles [7][8].
- **MRTR:** `InputRequiredResult` replaces server-initiated sampling, elicitation and roots requests. Roots, Sampling and Logging are deprecated [1].
- **Caching and prompt caching:** list and read results carry `ttlMs` and `cacheScope`. Servers "SHOULD return tools… in a deterministic order… [for] LLM prompt cache hit rates" [1].
- **Tracing:** OpenTelemetry trace context (`traceparent`) goes in `_meta` [1].
- **Authorization:**
  - OAuth 2.1 (draft 13).
  - Servers must publish Protected Resource Metadata (RFC 9728).
  - Client ID Metadata Documents are preferred; DCR is deprecated.
  - Clients must validate `iss` per RFC 9207.
  - The stdio transport "SHOULD NOT follow this specification, and instead retrieve credentials from the environment" [6].
- **HTTP safety:** servers must validate `Origin` against DNS rebinding and should bind to 127.0.0.1 when local [5].
- **Implication [Inf]:** hivemind's own API must provide cursors, handles and resume. Don't rely on MCP sessions or on SSE redelivery.

### 7.2 rmcp, the official Rust SDK [V] [11][12]

- Tier 1 in the official SDK list.
- rmcp-v3.9.0 (2026-11-05) "implements the stable MCP 2026-07-28 specification while remaining fully compatible with 2025-11-25". It covers discovery, subscriptions, tasks, caching, MRTR, and stateless Streamable HTTP.
- Releases are frequent: 3.5.0 on 2026-09-28 and 3.9.0 on 2026-11-05.
- **Implication [Inf]:** pin the version and keep rmcp behind a hivemind transport adapter.

### 7.3 What clients actually support

- Claude Code supports stdio, HTTP, SSE and WebSocket servers [21]. Codex supports stdio and Streamable HTTP with bearer tokens, OAuth, CIMD and DCR [22]. [V]
- The community-maintained extension matrix (updated 2026-11-03) marks Tasks only for Claude Code. VS Code Copilot, Cursor, Goose and ChatGPT are marked for MCP Apps [9]. [V]
- Client support for `subscriptions/listen` is not tracked in any matrix found. [?]

### 7.4 gRPC with tonic

- The repo moved to **grpc/grpc-rust**; tonic v0.14.9 was released 2026-10-29. "master branch is currently preparing breaking changes" [13]. [V]
- It supports bidirectional streaming, rustls, and health checking [13]. [V]
- **Use [Inf]:** a service-to-service and SDK API with server-streaming `Watch(cursor)`. MCP remains the interface for agents.

### 7.5 A2A

- v1.0.0 (2026-03-12) and v1.0.1 (2026-05-28), Apache-2.0 [14]. [V]
- Donated to the Linux Foundation, with a steering committee drawn from AWS, Cisco, Google, IBM Research, Microsoft, Salesforce, SAP and ServiceNow [15]. [V]
- Bindings for JSON-RPC, gRPC and HTTP+JSON, plus streaming and push notifications [14]. [V]
- **Use [Inf]:** not for v0. Record A2A task and artifact IDs in provenance so handoffs can link across systems.

### 7.6 OpenTelemetry GenAI semantic conventions

- They moved to `open-telemetry/semantic-conventions-genai` [20][16] and are in Development status [17]. [V]
- The `gen_ai.operation.name` values include `create_memory`, `search_memory`, `update_memory`, `upsert_memory`, `delete_memory`, `create_memory_store` and `retrieval` [17]. [V]
- Memory attributes: `gen_ai.memory.store.id`, `.record.id`, `.record.count`, `.query.text` [16]. [V]
- Token attributes: `gen_ai.usage.input_tokens`, `output_tokens`, `cache_read.input_tokens`, `cache_write.input_tokens`, `reasoning.output_tokens` [16]. Token metrics such as `gen_ai.client.inference.usage.*` [18]. [V]
- MCP metrics: `mcp.client.operation.duration` and `mcp.server.operation.duration` [19]. [V]
- **Use [Inf]:** emit spans with these names, and add a hivemind metric for tokens returned per retrieval. Pin the convention version, since Development status means names can change.

### 7.7 Protocol options (table)

| Interface | Buys | Costs | Status | Use in hivemind |
|---|---|---|---|---|
| MCP stdio | Works with every coding agent; no network exposure | One process per session (§4.2); no OAuth | Stable [4] | v0 shim to the daemon |
| MCP Streamable HTTP (2026-07-28) | Remote shared service; stateless, so it scales horizontally; OAuth 2.1 | Auth server integration; Origin/DNS-rebinding hardening | Stable [5][6] | v1 |
| MCP subscriptions / Tasks | Push changes; long-running jobs such as consolidation | Uneven client support [9] | Core / Beta extension [3][8] | Optional, with polling as the fallback |
| gRPC (tonic) | Typed API; server streaming; efficient | A second API surface; tonic master breaking | 0.14.9 [13] | v1 service API and SDKs |
| A2A | Agent↔agent task protocol | Different problem | v1.0.1 [14] | Store references only |
| OTel GenAI | Standard telemetry on token cost | Development status | [16] | From v0 |

---

## 8. Security and multi-tenancy

### 8.1 Evidence: memory poisoning is real

**Attacks on production memory features [I]:**
- **ChatGPT "SpAIware"** (2024-09-20): prompt injection written into long-term memory caused data exfiltration that persisted across sessions [102].
- **Gemini** (2025-02-10): poisoned memories via "delayed tool invocation" [103].
- **Windsurf** (2025-08-22): Cascade's `create_memory` tool "is automatically invoked", so injected instructions persist into future conversations [104].
- **Bedrock Agents** (Unit 42, 2025-10-09): indirect prompt injection "silently poison[s] the long-term memory" [105].

**Research results [I]:**
- **AgentPoison:** over 80% average attack success, under 1% impact on benign tasks, at a poison rate under 0.1% [106].
- **PoisonedRAG:** 90% success with five malicious texts per target question in a database of millions. "Several defenses… are insufficient" [108].
- **MINJA:** injects malicious memory records "by only interacting with the agent via queries" [107].

**Adjacent persistence vectors for coding agents [I]:**
- "Rules File Backdoor": hidden Unicode in Cursor/Copilot rules files (2025-03-18) [111].
- MCPoison (CVE-2025-54136): an approved MCP config, later modified, keeps running without new consent (2025-08-05) [112].
- MCP tool poisoning (2025-04-01) [109].
- GitHub MCP: a malicious public issue leads to a private-repo leak (2025) [110].

**Taxonomies [V]:** OWASP LLM04:2025 (data and model poisoning) [113] and LLM08:2025 (vector and embedding weaknesses, including cross-context leakage and inadequate access controls) [114]. OWASP's agentic threat and Top-10 resources exist [115][116]; their detailed contents were not verified here [?]. MCP's security best practices cover confused deputies, token passthrough, SSRF and local server compromise [10].

### 8.2 Controls [Inf unless cited]

- **Provenance and trust tiers.** Every event records its source class: human-typed, agent-inferred, tool output, repository file at a commit, web, or third party.
  - Trust propagates through `derived_from` as the minimum of the inputs ("taint").
  - Untrusted-source writes go to quarantine by default. They are excluded from default retrieval or returned wrapped and labeled as data.
  - Procedural memories (instructions such as "always do X") need a higher tier and explicit promotion.
  - Strip invisible and bidi Unicode at ingest (cf. [111]).
- **Attributed and signed writes.** Authenticate the principal per connection: OAuth over HTTP [6], `SO_PEERCRED` over the Unix socket [59], and environment-provided credentials for stdio [6]. Treat `clientInfo` as a label, not an identity [Inf]. Reserve Ed25519 signatures [118] for replication; iroh-docs' namespace and author keys are a precedent [79].
- **Scoped ACLs:** agent-private, project, team or shared. Filter by ACL **before** ranking, with tenant identity taken from authentication, never from client-supplied filters [90]. A summary built across scopes gets the intersection of their ACLs.
- **Secret redaction at ingest.** Store a secret-bearing payload only as redacted content plus a hash. Embed only after redaction.
- **Audit log.** Hash-chain the log and periodically checkpoint the head, transparency-log style [117]. This makes tampering evident; it doesn't prevent it.
- **Deletion guarantees.** Erase runs as a pipeline: log payload, then each projection acknowledges, then backups expire, then a verification query confirms. Offer crypto-shredding for sensitive classes [40].
- **Limits and fencing.** Per-principal quotas and rate limits. Diversity caps per author and source in result sets, against AgentPoison-style dominance of a query. Fencing tokens on leases [99].

### 8.3 Threat model (table)

| Threat | Mitigation | Residual risk |
|---|---|---|
| Indirect injection persisted as memory [102]–[105] | Provenance tags; quarantine untrusted sources; taint propagation; no automatic promotion of instructions | An agent can paraphrase untrusted text into a "self-authored" memory; taint rules must catch it |
| Retrieval-targeted poisoning [106][108] | Trust-weighted ranking; per-source diversity caps; detection of near-duplicate bursts; per-principal write limits | Optimized triggers may still win; published defenses are weak [108] |
| Query-only injection [107] | Provenance requirements and review for "lesson" and procedural memories; dedupe | The agent's own memory-writing logic remains the attack path |
| Hidden instructions (Unicode) [111] | Normalize and strip at ingest; show diffs on promotion | New encodings |
| Leakage across scopes or tenants [114] | ACL pre-filter driven by authentication [90]; per-tenant partitions | Inference via aggregates or summaries |
| Secrets stored in memory | Ingest scanning and redaction; erase pipeline | Scanner false negatives; secrets already in model provider logs |
| Unattributed or impersonated writes | Per-connection authentication; signatures in replication [118] | A compromised agent with valid credentials |
| Confused deputy or token passthrough [10] | Audience-bound tokens; no passthrough [6] | Misconfiguration |
| DNS rebinding on a local HTTP endpoint [5] | Prefer the Unix socket; Origin validation; bind 127.0.0.1 | Low |
| Log or audit tampering | Hash chain plus signed checkpoints [117] | Local root can still tamper |
| Incomplete deletion | Per-projection acknowledgements; crypto-shredding; documented backup retention windows | Copies outside hivemind (transcripts) |
| Stale lease holder | Fencing tokens [99] | External side effects that ignore the token |
| Flooding / denial of service | Quotas, size caps | Cost of tuning |
| Trust persistence of daemon or config [112] | Pin the daemon binary hash and version; re-consent on change | User fatigue |

---

## 9. Staged architecture recommendation

### v0: local (one machine) [Inf]

- **Process model:**
  - One static binary with three modes: `hivemindd` (daemon), `hivemind mcp` (stdio shim), and a CLI.
  - `hivemind-core` is also usable as an embedded library.
- **Store:**
  - SQLite WAL in the local data directory; refuse to run on network filesystems.
  - Tables: an `events` log; `blobs` keyed by content hash; projections (`heads`, FTS5, vectors, edges); projection offsets; idempotency; leases; audit.
  - Choose `synchronous` from measurements [119].
- **Write path:** a single writer task with group commit. Each write returns `{event_id, offset}`.
- **Read path:** reads accept `min_offset`. Results carry provenance, trust tier and offsets.
- **Coordination:** `claim/renew/release` with TTL and fencing tokens. `changes_since(cursor)`, plus optional MCP `subscriptions/listen` on `hivemind://` URIs.
- **Security:** everything in §8.2, with HTTP off by default.
- **Operations:** optional Litestream; OTel GenAI spans.
- **Move to v1 when any of these is observed:**
  - Two or more machines or people need the same project memory.
  - Cloud-sandboxed agents (Codex cloud, the Copilot coding agent) need access.
  - Policy or audit must be central.

### v1: shared service

- **Interfaces:** `hivemind serve` exposes MCP Streamable HTTP (2026-07-28, stateless) and tonic gRPC with `Watch(cursor)`. It acts as an OAuth 2.1 resource server with Protected Resource Metadata.
- **Storage:** one SQLite file per (tenant, project) partition, each with a single writer and Litestream to object storage. **Switch to Postgres** if any of these is measured:
  - per-partition write p99 or busy rates exceed budget under the target agent count;
  - a managed HA database is required;
  - queries must span partitions.
- **Laptop daemons as replicas:** they push their own events through an idempotent upload queue (the PowerSync pattern [73]) and pull by cursor. Offline reads work and offline writes are queued. Leases require being online.
- **Move to v2 when one of these holds:**
  - An availability SLO can't be met by one node plus a Litestream restore, measured as RTO/RPO including deploys.
  - The working set exceeds one node.
  - Write throughput exceeds one writer per partition even after batching.
  - Cross-region p95 retrieval latency exceeds budget.
  - A tenant requires dedicated infrastructure.

### v2+: distributed

- **Option A, Raft per partition** (openraft or hiqlite-style): HA, and leases are linearizable by construction. It costs three times the resources, membership operations, and exposure to a pre-1.0 API.
- **Option B, object-storage-native:** per-partition WAL segments committed with conditional-write CAS [88], stateless query nodes with NVMe cache, asynchronous indexers, and exhaustive search of the unindexed tail [86]. A small Raft group or the CAS itself holds leases. It costs 100 ms-scale writes and cold-cache latency.
- **Option C, external engines as projections:** Qdrant, Weaviate or Postgres+pgvector fed from the log, which stays authoritative.
- **Default plan:** B for data and A only for metadata and leases. Do this only when a trigger fires.

### Design these into v0 so later stages aren't blocked

1. UUIDv7 event IDs, a BLAKE3 content hash, and `(origin_id, origin_seq)`.
2. A per-partition offset used as the universal cursor in every API.
3. Idempotency keys on every write, retained at least as long as the longest offline window.
4. A partition key on every event and API; no transactions across partitions.
5. Canonical, versioned serialization of events (needed for hashing and signing).
6. Projections that are pure functions of the log plus a version (embedding model ID); a rebuild command.
7. Payloads stored separately from envelopes.
8. Actor and provenance fields required; `sig`, `hlc` and `key_id` reserved.
9. Leases with fencing tokens from day one.
10. A core service layer independent of transport, with MCP, gRPC and CLI adapters.
11. No reliance on MCP sessions or resumability.

---

## 10. Open questions

- Which coding-agent clients implement `subscriptions/listen` and the Tasks extension today? The matrix only marks Claude Code for Tasks [9]. [?]
- Do agent sandboxes allow connecting to a Unix socket or spawning a daemon? [?]
- How should a stdio shim get a per-agent identity when stdio uses environment credentials [6]?
- At what partition write rate does single-writer SQLite stop meeting the p99 budget? This needs a hivemind benchmark harness.
- Where does the embedding model run (daemon or remote)? That decides the RAM argument for the daemon.
- How should taint propagate through summaries and consolidation, so that untrusted content can't be laundered into trusted memory?
- What is the erasure SLA, and is crypto-shredding legally sufficient? [?]
- Should OTel GenAI memory conventions be adopted now, given Development status?
- When do redb's `MultiWriter` mode or Turso's multi-process support become credible alternatives?
- How should signing keys for agents be distributed and rotated?

---

## 11. Sources (all checked 2026-11-10)

1. MCP 2026-07-28 changelog — https://modelcontextprotocol.io/specification/2026-07-28/changelog
2. MCP spec releases — https://github.com/modelcontextprotocol/modelcontextprotocol/releases
3. MCP Subscriptions — https://modelcontextprotocol.io/specification/2026-07-28/basic/patterns/subscriptions
4. MCP stdio transport — https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/stdio
5. MCP Streamable HTTP — https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http
6. MCP Authorization — https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization
7. MCP Extensions overview — https://modelcontextprotocol.io/extensions/overview
8. MCP Tasks extension — https://modelcontextprotocol.io/extensions/tasks/overview
9. MCP extension client matrix (source) — https://github.com/modelcontextprotocol/modelcontextprotocol/blob/main/docs/extensions/client-matrix.mdx
10. MCP Security Best Practices — https://modelcontextprotocol.io/docs/2026-07-28/tutorials/security/security_best_practices
11. MCP SDKs and tiers — https://modelcontextprotocol.io/docs/2026-07-28/sdk
12. rmcp (README, rmcp-v3.9.0) — https://github.com/modelcontextprotocol/rust-sdk
13. tonic / grpc-rust — https://github.com/grpc/grpc-rust
14. A2A releases and spec — https://github.com/a2aproject/A2A/releases ; https://github.com/a2aproject/A2A/blob/main/docs/specification.md
15. A2A governance — https://a2a-protocol.org/latest/
16. OTel GenAI semconv repo and attribute registry — https://github.com/open-telemetry/semantic-conventions-genai
17. OTel GenAI metrics — https://github.com/open-telemetry/semantic-conventions-genai/blob/main/docs/gen-ai/gen-ai-metrics.md
18. OTel GenAI token metrics — https://github.com/open-telemetry/semantic-conventions-genai/blob/main/docs/gen-ai/gen-ai-token-metrics.md
19. OTel MCP conventions — https://github.com/open-telemetry/semantic-conventions-genai/blob/main/docs/gen-ai/mcp.md
20. OTel core "Moved" notice — https://github.com/open-telemetry/semantic-conventions/blob/main/docs/gen-ai/README.md
21. Claude Code MCP docs — https://code.claude.com/docs/en/mcp
22. Codex MCP docs — https://developers.openai.com/codex/mcp
23. "Datomic is Free" (2023-04-27) — https://blog.datomic.com/2023/04/datomic-is-free.html
24. Datomic filters — https://docs.datomic.com/reference/filters.html
25. Datomic excision — https://docs.datomic.com/reference/excision.html
26. Datomic ACID — https://docs.datomic.com/transactions/acid.html
27. Jepsen: Datomic Pro 1.0.7075 — https://jepsen.io/analyses/datomic-pro-1.0.7075
28. XTDB README — https://github.com/xtdb/xtdb
29. XTDB SQL transactions — https://docs.xtdb.com/reference/main/sql/txs.html
30. Time in XTDB — https://docs.xtdb.com/about/time-in-xtdb.html
31. Kulkarni & Michels, "Temporal features in SQL:2011" — https://sigmodrecord.org/publications/sigmodRecord/1209/pdfs/07.industry.kulkarni.pdf
32. PostgreSQL 18 release notes — https://www.postgresql.org/docs/release/18.0/
33. MariaDB system-versioned tables — https://mariadb.com/docs/server/reference/sql-structure/temporal-tables/system-versioned-tables
34. TerminusDB — https://github.com/terminusdb/terminusdb
35. Dolt — https://github.com/dolthub/dolt
36. Dolt merge docs — https://docs.dolthub.com/concepts/dolt/git/merge
37. Fowler, Event Sourcing — https://martinfowler.com/eaaDev/EventSourcing.html
38. Stripe idempotent requests — https://docs.stripe.com/api/idempotent_requests
39. Verraes, Forgettable Payloads — https://verraes.net/2019/05/eventsourcing-patterns-forgettable-payloads/
40. Verraes, Crypto-Shredding — https://verraes.net/2019/05/eventsourcing-patterns-throw-away-the-key/
41. KurrentDB (license) and appending docs — https://github.com/kurrent-io/KurrentDB ; https://docs.kurrent.io/clients/dotnet/v1.4/appending-events.html
42. LanceDB guarantees — https://docs.lancedb.com/basics/guarantees
43. SQLite WAL — https://sqlite.org/wal.html
44. SQLite over a network — https://sqlite.org/useovernet.html
45. SQLite transactions — https://sqlite.org/lang_transaction.html
46. SQLite result codes — https://sqlite.org/rescode.html
47. sqlite3_busy_timeout — https://sqlite.org/c3ref/busy_timeout.html
48. SQLite BEGIN CONCURRENT — https://sqlite.org/src/doc/begin-concurrent/doc/begin_concurrent.md
49. LMDB docs — http://www.lmdb.tech/doc/
50. redb README and CHANGELOG (4.5.0) — https://github.com/cberner/redb ; https://github.com/cberner/redb/blob/master/CHANGELOG.md
51. fjall README — https://github.com/fjall-rs/fjall
52. RocksDB read-only and secondary instances — https://github.com/facebook/rocksdb/wiki/Read-only-and-Secondary-instances
53. DuckDB concurrency — https://duckdb.org/docs/2.0/connect/concurrency.html
54. Turso README — https://github.com/tursodatabase/turso
55. sccache README and configuration — https://github.com/mozilla/sccache ; https://github.com/mozilla/sccache/blob/main/docs/Configuration.md
56. Ollama FAQ — https://github.com/ollama/ollama/blob/main/docs/faq.mdx
57. Docker daemon socket — https://docs.docker.com/engine/security/protect-access/
58. MCP memory server PR #4555, issue #4797, commit 7be08e9 — https://github.com/modelcontextprotocol/servers/pull/4555 ; https://github.com/modelcontextprotocol/servers/issues/4797
59. unix(7) — https://man7.org/linux/man-pages/man7/unix.7.html
60. Litestream — https://github.com/benbjohnson/litestream
61. Fly.io, "Litestream: Revamped" (2025-05-20) — https://fly.io/blog/litestream-revamped/
62. Fly.io, "Litestream v0.5.0 is Here" (2025-10-02) — https://fly.io/blog/litestream-v050-is-here/
63. Fly.io, "Litestream VFS" (2025-12-11) — https://fly.io/blog/litestream-vfs/
64. LiteFS — https://github.com/superfly/litefs
65. Turso embedded replicas — https://docs.turso.tech/features/embedded-replicas/introduction
66. Turso sync conflict resolution — https://docs.turso.tech/sync/conflict-resolution
67. Turso AgentFS — https://docs.turso.tech/agentfs/introduction
68. rqlite and read consistency — https://github.com/rqlite/rqlite ; https://rqlite.io/docs/api/read-consistency/
69. dqlite — https://github.com/canonical/dqlite
70. cr-sqlite — https://github.com/vlcn-io/cr-sqlite
71. ElectricSQL — https://github.com/electric-sql/electric
72. PowerSync service — https://github.com/powersync-ja/powersync-service
73. PowerSync consistency — https://docs.powersync.com/architecture/consistency
74. jepsen-powersync — https://github.com/nurturenature/jepsen-powersync
75. Automerge 3 — https://automerge.org/blog/automerge-3/
76. Loro — https://github.com/loro-dev/loro
77. y-crdt (yrs) — https://github.com/y-crdt/y-crdt
78. iroh — https://github.com/n0-computer/iroh
79. iroh-docs — https://github.com/n0-computer/iroh-docs
80. iroh-blobs — https://github.com/n0-computer/iroh-blobs
81. Ink & Switch, Local-first software — https://www.inkandswitch.com/local-first/
82. openraft — https://github.com/databendlabs/openraft
83. raft-rs — https://github.com/tikv/raft-rs
84. hiqlite — https://github.com/sebadob/hiqlite
85. SlateDB — https://github.com/slatedb/slatedb
86. turbopuffer architecture — https://turbopuffer.com/docs/architecture
87. WarpStream architecture — https://docs.warpstream.com/warpstream/overview/architecture
88. "Amazon S3 now supports conditional writes" — https://aws.amazon.com/about-aws/whats-new/2024/08/amazon-s3-conditional-writes/
89. Qdrant consistency guarantees — https://qdrant.tech/documentation/scaling/consistency-guarantees/
90. Qdrant scaling, multitenancy and llms.txt — https://qdrant.tech/documentation/scaling/ ; https://qdrant.tech/documentation/guides/multiple-partitions/ ; https://qdrant.tech/llms.txt
91. Weaviate consistency — https://docs.weaviate.io/weaviate/concepts/replication-architecture/consistency
92. Weaviate multi-tenancy — https://docs.weaviate.io/weaviate/concepts/data
93. Milvus v3.0.0 release — https://github.com/milvus-io/milvus/releases/tag/v3.0.0
94. Chroma architecture — https://docs.trychroma.com/reference/architecture/overview ; https://docs.trychroma.com/reference/architecture/distributed
95. Jepsen analyses index — https://jepsen.io/analyses
96. Jepsen: NATS 2.12.1 — https://jepsen.io/analyses/nats-2.12.1
97. Jepsen, read-your-writes — https://jepsen.io/consistency/models/read-your-writes
98. Jepsen, causal — https://jepsen.io/consistency/models/causal
99. Kleppmann, "How to do distributed locking" — https://martin.kleppmann.com/2016/02/08/how-to-do-distributed-locking.html
100. RFC 9562 (UUIDv7) — https://www.rfc-editor.org/rfc/rfc9562
101. Kulkarni et al., Hybrid Logical Clocks — https://cse.buffalo.edu/tech-reports/2014-04.pdf
102. Embrace The Red, ChatGPT SpAIware (2024-09-20) — https://embracethered.com/blog/posts/2024/chatgpt-macos-app-persistent-data-exfiltration/
103. Embrace The Red, Gemini memory (2025-02-10) — https://embracethered.com/blog/posts/2025/gemini-memory-persistence-prompt-injection/
104. Embrace The Red, Windsurf SpAIware (2025-08-22) — https://embracethered.com/blog/posts/2025/windsurf-spaiware-exploit-persistent-prompt-injection/
105. Unit 42, "When AI Remembers Too Much" (2025-10-09) — https://unit42.paloaltonetworks.com/indirect-prompt-injection-poisons-ai-longterm-memory/
106. AgentPoison — https://arxiv.org/abs/2407.12784
107. MINJA — https://arxiv.org/abs/2503.03704
108. PoisonedRAG — https://arxiv.org/abs/2402.07867
109. Invariant Labs, Tool Poisoning (2025-04-01) — https://invariantlabs.ai/blog/mcp-security-notification-tool-poisoning-attacks
110. Invariant Labs, GitHub MCP exploit — https://invariantlabs.ai/blog/mcp-github-vulnerability
111. Pillar Security, Rules File Backdoor (2025-03-18) — https://www.pillar.security/blog/new-vulnerability-in-github-copilot-and-cursor-how-hackers-can-weaponize-code-agents
112. Check Point, MCPoison CVE-2025-54136 (2025-08-05) — https://research.checkpoint.com/2025/cursor-vulnerability-mcpoison/
113. OWASP LLM04:2025 — https://genai.owasp.org/llmrisk/llm042025-data-and-model-poisoning/
114. OWASP LLM08:2025 — https://genai.owasp.org/llmrisk/llm082025-vector-and-embedding-weaknesses/
115. OWASP Agentic AI Threats and Mitigations — https://genai.owasp.org/resource/agentic-ai-threats-and-mitigations/
116. OWASP Top 10 for Agentic Applications 2026 — https://genai.owasp.org/resource/owasp-top-10-for-agentic-applications-for-2026/
117. RFC 9162 (Certificate Transparency 2.0) — https://www.rfc-editor.org/rfc/rfc9162
118. RFC 8032 (Ed25519) — https://www.rfc-editor.org/rfc/rfc8032
119. SQLite PRAGMA synchronous — https://sqlite.org/pragma.html#pragma_synchronous

*Maturity data (latest release, license, archived status, last push) came from read-only `gh` queries on 2026-11-10. None of the repositories checked were archived.*
