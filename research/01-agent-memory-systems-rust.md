# Research 01: Purpose-built agent memory systems (Rust-native first)

> Research thread report, checked 2026-11-10. Produced by a research agent from public sources and saved verbatim. Evidence labels: **[V]** verified in a primary source, **[C]** project or vendor claim, **[I]** independent evidence, **[Inf]** inference, **[?]** unknown. Synthesis and decisions live in [`docs/`](../docs/).

## Executive summary

- **No Rust-native agent-memory system is ready to adopt as hivemind's core.** None combines a mature engine, safe concurrent multi-agent writes, verified provenance and time-validity semantics, and MCP plus a remote API.
- **The best semantics are attached to small or quiet projects:**
  - **Aionforge Memory** (jscott3201) fits coding agents most closely: append-only episodes, agent-private/team/global namespaces, work items, agent-to-agent messaging, provenance, no LLM calls when writing, and MCP over stdio and Streamable HTTP. But it has one maintainer, no commits since 2026-07-11, and depends on that maintainer's own 4-star graph engine (selene-db).
  - **KIP 2.0 / Cognitive Nexus on AndaDB** (ldclabs) has the most rigorous model of what is believed, when, and on whose evidence. It separates claims from who asserted them, tracks "when true" separately from "when recorded", distinguishes three kinds of correction, keeps permissions out of reach of memory content, and has a crash test that simulates power loss at every write point. It also has 1–3 contributors, a single-writer store, breaking upgrades, and a query engine with no semantic search. Its MCP layer (Anda Brain) relies on LLM agents to form memories.
- **Statelet makes broad claims that can't be checked.** It claims Raft, CDC, temporal graphs and 91.6% on LongMemEval-S, but the engine source isn't public ("coming soon"), and its licence signals conflict (Fair Source FSL on the site vs Apache-2.0 in the Homebrew formula).
- **Memorose: avoid.** It's LLM-heavy, has no MCP, has had no commits since June 2026, and writes to RocksDB without forcing data to disk — including the votes and log of its claimed Raft replication.
- **HelixDB is a database, not a memory system.** Its own docs put extraction, deciding updates and expiry in application code. Its MCP endpoint is Helix Cloud only, with OAuth.
- **memvid has 17k stars but is a single-writer file format.** It had checksum-mismatch bugs in its write-ahead log in 2026, fixes for which are still unmerged (#231, #249), has had no commits since July 2026, and its "+35% SOTA on LoCoMo" is the vendor's own number.
- **cortex-mem is a wrapper:** markdown files plus an external Qdrant vector database plus LLM calls, MCP over stdio only, and a LoCoMo evaluation on a single conversation.
- **shodh-memory** (no LLM calls, RocksDB) and **huggingface/funes** (an append-only index of past agent sessions with exact source references, no LLM, shareable through the Hugging Face Hub) are the most credible "local, cheap, deterministic" designs. funes is active and backed by an organisation, and is usable now for recalling past sessions.
- **No independent benchmarks were found for any candidate.** Every LoCoMo or LongMemEval number is self-reported.

**Recommendation:** build a thin hivemind layer rather than adopt one of these.
- Borrow the data model from KIP (claims vs assertions, two time axes, kinds of correction, permissions outside content) and Aionforge (namespaces, work items, handoffs, recall returned as untrusted text).
- Follow funes and shodh in keeping LLMs off the write path by default.
- Put it on a mature storage substrate chosen by the other threads.
- Use funes as the baseline to beat on outcomes per token.

**What would change this:**
- Aionforge resumes development and gains contributors.
- Statelet releases its source and passes independent evaluation.
- AndaDB or the Nexus adds safe multi-writer support and semantic search.

---

## Rust-native agent memory systems: survey for hivemind (checked 2026-11-10)

## 1. Summary

1. **No candidate is ready for "use as-is" as hivemind's core.** None combines a mature, multi-contributor engine with safe multi-writer access, verified provenance and two time axes, and MCP plus a remote API. **[Inf]** from the evidence below.
2. **Aionforge Memory** is the closest fit for coding-agent shared memory **[V]**:
   - immutable episodes;
   - derived facts linked by `SUPERSEDED_BY` / `CONTRADICTS` edges;
   - agent-private/team/global/system namespaces;
   - work items, a durable agent-to-agent pager, and audit records;
   - no LLM on the write path;
   - MCP over stdio and Streamable HTTP;
   - a write-ahead log (WAL) that flushes to disk (fsync) before commits become visible.

   Against that: one contributor, no commits since 2026-07-11, a niche single-maintainer engine (selene-db), no snapshotting or WAL truncation, and no benchmarks.
3. **KIP 2.0 / Cognitive Nexus (on AndaDB)** has the most rigorous memory model **[V]**:
   - a truth-neutral Proposition is separate from each actor's Assertion about it;
   - "what was true then" (`FOR TIME`) is separate from "what the store held then" (`AS OF SEQ`);
   - three different corrections: the world changed (temporal succession), the claim was wrong (supersession), the extraction was wrong (recording repair);
   - a governance plane that memory content cannot reach;
   - export/import capsules.

   But the query engine refuses semantic search, has no atomic batches and no cross-Space sharing. AndaDB itself is single-writer and persists on a flush schedule.
4. **AndaDB's crash test is the best durability evidence in this group.** It simulates a power failure after every possible write and checks a stated recovery contract **[V]**.
5. **Anda Brain** wraps the Nexus with MCP (stdio and Streamable HTTP) **[V]**. Writes go through LLM "Formation" agents (DeepSeek by default), and it has one contributor.
6. **Statelet** claims Raft, a change-data-capture (CDC) feed, AS-OF queries, superseding facts and 91.6% on LongMemEval-S **[C]**. The engine source is not public **[V]**. Only clients, a benchmark harness and binaries are published. Licence signals conflict.
7. **Memorose** claims Raft, multimodal memory and L0–L3 layers. In code, RocksDB uses default write options (logged but not fsynced), including for Raft votes and log entries **[V]**. It is LLM-dependent, has no MCP (deliberately), and has been dormant since 2026-06-07.
8. **HelixDB** positions itself as the database half of agent memory. Its own guide puts extraction, update decisions and expiry in the application **[V]**. Its MCP endpoint is Helix Cloud only (OAuth, with a two-step confirm for writes) **[V]**. It is very active (v3.6.2 released 2026-11-10).
9. **memvid v2** is a solid single-file embedded format **[V]**: WAL records with blake3 checksums, fsync after each append, an OS-level exclusive writer lock, and "memory cards" with version relations. But it is single-writer (open feature request #218), had a WAL checksum bug in 2026 (#230) with WAL fixes unmerged (#231, #249), has had no commits since 2026-07-14, and its LoCoMo claims are the vendor's own.
10. **cortex-mem** is a wrapper: markdown files plus an external Qdrant plus LLM extraction, MCP over stdio only **[V]**. Its LoCoMo evaluation covers one conversation (152 questions) **[V]**.
11. **shodh-memory** makes no LLM calls on writes or recall and runs ONNX embedding, reranking and named-entity models locally on RocksDB, with 51 MCP tools **[V/C]**. By default it does not fsync (survives a process crash, not power loss) **[V]**. A body-truncation data-loss bug (#391) was fixed **[V]**.
12. **huggingface/funes** is an append-only, deterministic index of agent transcripts **[V]**: a Lance dataset, vector plus BM25 search with cross-encoder reranking, exact `session_id`+`turn_uuid` source references, sharing via Hugging Face Hub datasets, and read-only stdio MCP. It is the strongest existing baseline for "remember past coding sessions" with no LLM spend.
13. **No independent third-party benchmark of any candidate was found [?].** All LoCoMo/LongMemEval figures are vendor-run, and several measure retrieval rather than question-answering accuracy.
14. **Many repos appeared in 2026 with fast-growing star counts but one to three contributors.** Star counts are not a useful maturity signal here **[V]** (metadata).
15. **Recommendation:** build a thin hivemind memory layer that borrows the KIP, Aionforge and funes ideas, on a mature storage substrate (chosen by the other threads). Use funes and shodh-memory as cost/quality baselines. Re-check Aionforge and Statelet in about three months.

---

## 2. Method and scope

- Repo metadata was pulled with `gh api` on 2026-11-10: licence, created/pushed dates, latest release, contributor count (first 100 including anonymous), commits on the default branch since 2026-08-12 (about 90 days), and open issue counts.
- READMEs, docs and selected source files (storage, WAL, locking, MCP wiring) were read through the GitHub contents API at pinned commits. Nothing was cloned, built or run.
- Discovery used GitHub repo search (keyword plus `language:rust`, and topics `agent-memory`, `ai-memory`, `llm-memory`, `long-term-memory`, `agentic-memory`).
- The issue-search API hit a rate limit partway through. Durability-issue sweeps are complete for memvid, shodh-memory and cortex-mem, and partial for the others **[?]**.

Labels: **[V]** verified in a primary source · **[C]** project claim · **[I]** independent third party · **[Inf]** inference · **[?]** unknown.

---

## 3. Candidates

### 3.1 AndaDB, Anda framework, KIP and Anda Brain (ldclabs)

**Identity and maturity [V]**

| Repo | Role | Licence | Created | Last push | Release | Contributors | Commits since 2026-08-12 |
|---|---|---|---|---|---|---|---|
| ldclabs/anda-db | Embedded DB, KIP SDK, Cognitive Nexus, servers | MIT | 2025-03-28 | 2026-10-26 | crates only, "0.14 release family"; no GitHub Releases | 3 | 194 |
| ldclabs/KIP | Protocol spec plus conformance suite | MIT | 2025-06-09 | 2026-10-26 | kip-lang-v2.1.0 (2026-08-30) | 1 | 60 |
| ldclabs/anda | Agent framework | MIT + Apache-2.0 licence files | 2025-01-03 | 2026-10-20 | v0.15.0 (2026-08-07) | 5 | 51 |
| ldclabs/anda-brain | Memory service product (Formation/Recall/Maintenance, MCP) | Apache-2.0 | 2026-03-11 | 2026-10-17 | v0.13.5 (2026-10-17) | 1 | n/a |

**What it is.** A layered stack, not a single product:
- **AndaDB** is a database primitive: an embedded document store with B-Tree, BM25 and HNSW indexes and reciprocal-rank fusion (RRF) for hybrid search, persisted through Rust's `object_store` (local filesystem, S3, GCS, Azure) **[V]** ([README L78-L102](https://github.com/ldclabs/anda-db/blob/8ad402736559eba06b81a71f23612d0eff0fd64b/README.md#L78-L102)).
- **Cognitive Nexus** is a reference engine for **KIP 2.0**, a protocol with three parts: a query language (KQL), a mutation language (KML) and META commands.
- **Anda Brain** is the agent-facing memory service. It uses LLM agents to translate natural language into KIP.

**Data model (Nexus/KIP) [V]** ([nexus doc L128-L157](https://github.com/ldclabs/anda-db/blob/8ad402736559eba06b81a71f23612d0eff0fd64b/docs/anda_cognitive_nexus.md#L128-L157); [KIP README L44-L58](https://github.com/ldclabs/KIP/blob/c4f3c17a1306d89562f3778af0013f0de17436a0/README.md#L44-L58))
- **Element kinds:** Concept, Proposition (a truth-neutral subject/predicate/object triple), Assertion (one actor's stance, mode, confidence, evidence and valid time), Evidence, Activity.
- **Belief is computed at read time**, under a named, versioned policy. It is never stored.
- **Two time axes.** `FOR TIME` (valid time) vs `AS OF SEQ` (record time). An `element_versions` table appends whole rows in the same commit as the change ([nexus doc §12](https://github.com/ldclabs/anda-db/blob/8ad402736559eba06b81a71f23612d0eff0fd64b/docs/anda_cognitive_nexus.md#L721-L734)).
- **Three kinds of correction:** temporal succession (the world changed), supersession (the claim was wrong), recording repair (the extraction was wrong).
- **Rules:**
  - "Repetition is not corroboration": assertions are grouped, and shared evidence merges groups.
  - Retention expiry is kept separate from valid-time expiry.
  - Memory strength decays on a 30-day half-life, computed on read.
- **Scopes:** Spaces. **Governance:** principals, grants, delegations, policies and audit live in eight collections "reachable from no KML clause", so memory content cannot grant itself privileges ([L558-L697](https://github.com/ldclabs/anda-db/blob/8ad402736559eba06b81a71f23612d0eff0fd64b/docs/anda_cognitive_nexus.md#L558-L697)).
- **Erasure:** `PURGE` with legal hold and digest stubs.
- **Capsules:** export, verify and import with merge or quarantine. Capsules are not signed.

**Documented gaps [V]** ([L797-L813](https://github.com/ldclabs/anda-db/blob/8ad402736559eba06b81a71f23612d0eff0fd64b/docs/anda_cognitive_nexus.md#L797-L813)):
- The engine refuses semantic or hybrid `SEARCH`.
- No atomic batches.
- No cross-Space sharing.
- No retention policy per Space.

**Write path.**
- *Nexus:* one KML statement is one transaction. Atomic visibility comes from an in-process `RwLock`, not from AndaDB **[V]** ([L252-L286](https://github.com/ldclabs/anda-db/blob/8ad402736559eba06b81a71f23612d0eff0fd64b/docs/anda_cognitive_nexus.md#L252-L286)).
- *Anda Brain:* an asynchronous Formation agent reads conversations and writes evidence and claims. Maintenance "sleep" cycles consolidate, resolve identity and run revisions **[C/V]** ([anda-brain README L79-L87](https://github.com/ldclabs/anda-brain/blob/4d4d7faa3bc1947bdc55920d64a4d497377607c2/README.md#L79-L87)). The default model setup targets DeepSeek **[V]**.
- LLM calls per write are not documented, but Formation is an agent loop, so more than one call **[?]**/**[Inf]**.
- Graph size is capped by tier: 10^(n+2) Concepts **[V]** (L290-L292).

**Retrieval.**
- *AndaDB:* B-Tree filters, BM25 and HNSW, fused with RRF **[V]**.
- *Nexus:* graph patterns, multi-hop paths, keyword `SEARCH` only, and a work budget of 100k intermediate rows **[V]**.
- *Brain:* natural-language recall through an LLM planner, an optional token budget, a model-free "probe" **[C]**.

**Storage and durability [V]** ([anda_db.md L428-L549](https://github.com/ldclabs/anda-db/blob/8ad402736559eba06b81a71f23612d0eff0fd64b/docs/anda_db.md#L428-L549))
- "Persistence is incremental rather than transactional". Index state is flushed explicitly or by `auto_flush`.
- Updates and removals write durable intent records; inserts are covered by an allocation watermark (one extra small write per 64 inserts).
- Recovery happens only on reopen.
- **Single writer per database.** A cancelled write future "poisons" the handle until reopen.
- Optional AES-256-GCM encryption at rest **[C]**.
- **Crash test:** `crash_at_every_mutation_point_is_recoverable` simulates power failure after every possible write and checks that acknowledged flushes survive byte for byte **[V]** ([crash_recovery.rs L1-L16](https://github.com/ldclabs/anda-db/blob/8ad402736559eba06b81a71f23612d0eff0fd64b/rs/anda_db/tests/crash_recovery.rs#L1-L16), L257).

**Concurrency and deployment [V]**
- Embedded crate; `anda_cognitive_nexus_server` speaks HTTP/JSON-RPC (`POST /kip`).
- The server holds an advisory `.anda-nexus-writer.lock`.
- Multi-operation `atomic` requests are refused ([server README L55-L58, L96-L99](https://github.com/ldclabs/anda-db/blob/8ad402736559eba06b81a71f23612d0eff0fd64b/rs/anda_cognitive_nexus_server/README.md#L55-L99)).
- A shard proxy routed through PostgreSQL exists as source only **[V]**; it was not evaluated **[?]**.

**Interfaces.**
- *anda-db:* Rust, Python (PyO3), Cloudflare Durable Objects (TypeScript), HTTP/JSON-RPC, no MCP **[V]**.
- *Anda Brain:* REST and MCP over stdio and Streamable HTTP (`/mcp/{space_id}`) via `rmcp` **[V]**. Tools include `anda_brain_remember_conversation` and `anda_brain_recall_memory` ([mcp.rs](https://github.com/ldclabs/anda-brain/blob/4d4d7faa3bc1947bdc55920d64a4d497377607c2/anda_brain/src/mcp.rs#L1441-L1470)).
- KIP's own MCP server is archived at v1 **[V]** ([KIP README L27](https://github.com/ldclabs/KIP/blob/c4f3c17a1306d89562f3778af0013f0de17436a0/README.md#L27)).

**Auth and multi-tenancy.** Brain uses Ed25519-signed CWT tokens and per-Space tokens (`read`/`write`/`*`) **[C]** (L184). The Nexus has a full principal and grant model **[V]**. No change notifications in the Nexus server **[?]**; KIP describes a "Change Stream" **[C]**.

**Benchmarks.**
- AndaDB microbenchmarks on an Apple M1 Pro with the in-memory backend: about 0.002 ms for a filtered page and about 30 ms for a full 1M-ID set **[V]** ([benchmark README](https://github.com/ldclabs/anda-db/blob/8ad402736559eba06b81a71f23612d0eff0fd64b/docs/benchmarks/anda_query_scale_2026-09-26-rerun/README.md)). These are not agent-quality benchmarks.
- The KIP README says "No learning result is claimed" **[V]** (L15).
- No LoCoMo or LongMemEval results were found **[?]**.

**Verdict: borrow ideas** (the KIP data model, the governance/content separation, the crash-test methodology, capsules). Do not adopt as the core in v1. Reasons:
- one to three contributors;
- breaking upgrades between 0.x releases ("do not reopen with 0.14.0 afterwards");
- single writer with flush-based durability;
- no semantic search in the Nexus;
- the LLM-heavy Brain is at odds with hivemind's compute and token goals.

### 3.2 Aionforge Memory (jscott3201/aionforge-memory)

**Identity and maturity [V].**
- Apache-2.0/MIT, created 2026-06-06.
- v0.4.0 released 2026-07-11; last commit on `development` 2026-07-11 (`cd5aa355`).
- 1 contributor, 12 stars, 0 commits since 2026-08-12.
- Engine: selene-db (same author, 4 stars, created 2026-05, v2.0.0-alpha.1 2026-09-13).

**What it is.** A real agent-memory system ("substrate") on an embedded graph engine with ISO GQL. It is MCP-only: no web UI ([README L23](https://github.com/jscott3201/aionforge-memory/blob/cd5aa35512ef171ddf09b89426c69f13351b5ccf/README.md#L23)).

**Data model [V]** ([data-model.md L11-L15, L120-L141](https://github.com/jscott3201/aionforge-memory/blob/cd5aa35512ef171ddf09b89426c69f13351b5ccf/docs/data-model.md#L11-L141))
- 20 node labels. Memory-bearing: Episode (immutable), Fact (subject/predicate/object with confidence and status), Entity, Note, Skill, BadPattern, CoreBlock.
- Supporting nodes: Agent, MemSession, WorkItem, Message, ProvenanceRecord (source, transport, signature, trust), AuditEvent, Promotion, Scope, ValidityAnchor.
- Fact↔Fact edges: `SUPPORTS`, `SUPERSEDED_BY`, `CONTRADICTS`. Valid-time and record-time live on the `ABOUT` edge.
- Every node has `ingested_at`, `namespace`, `expired_at`. Memory nodes also have importance, trust, last access and a pinned flag.
- Namespaces: agent-private, team, global, system (README L75).
- **No file, URI or path pointers in any node** (L15). This is a limitation for coding-agent provenance.

**Write path [V].** Capture → sanitisation (redaction, injection markers) → one immutable Episode → **deterministic, rule-based** consolidation (fact extraction, summarisation, skill induction). "No chat/completion model is called anywhere in the substrate" ([honest-scope L20-L25](https://github.com/jscott3201/aionforge-memory/blob/cd5aa35512ef171ddf09b89426c69f13351b5ccf/docs/honest-scope.md#L20-L25)). Optional embeddings come from one OpenAI-compatible endpoint. Signed writes (Ed25519) are optional.

**Retrieval [V/C].** Lexical plus vector plus graph expansion plus recency, importance and trust. Louvain community-diversity cap and PageRank prior were tuned on small synthetic fixtures ([Benchmarks.md](https://github.com/jscott3201/aionforge-memory/blob/cd5aa35512ef171ddf09b89426c69f13351b5ccf/Benchmarks.md)). Results come back as a bounded text bundle, explicitly marked untrusted.

**Storage and durability.**
- WAL-backed selene-db; recovery replays the WAL and rebuilds indexes **[V]**.
- No snapshotting or WAL truncation, so erased values persist in the WAL **[V]** ([honest-scope L39-L40](https://github.com/jscott3201/aionforge-memory/blob/cd5aa35512ef171ddf09b89426c69f13351b5ccf/docs/honest-scope.md#L39-L40)).
- A historical 0.1.0 incident: WAL accepted live but failed replay after restart **[V]** ([operations-recovery L140-L145](https://github.com/jscott3201/aionforge-memory/blob/cd5aa35512ef171ddf09b89426c69f13351b5ccf/docs/operations-recovery.md#L140-L145)).
- selene-db v1.4.0 `SyncPolicy`: group commit with "fsync-before-publish" **[V]** ([writer.rs L27-L51 @v1.4.0](https://github.com/jscott3201/selene-db/blob/v1.4.0/crates/selene-persist/src/writer.rs#L27-L51)). Aionforge pins 1.3.0, so this is **[Inf]** for 1.3.

**Deployment and interfaces [V].**
- One binary (`doctor`, `recover`, `serve`), a Rust crate, and Docker on GHCR.
- MCP via `rmcp` 1.6 with `transport-io` and `transport-streamable-http-server` ([aionforge-mcp Cargo.toml](https://github.com/jscott3201/aionforge-memory/blob/cd5aa35512ef171ddf09b89426c69f13351b5ccf/crates/aionforge-mcp/Cargo.toml)).
- OAuth bearer validation is optional and off by default (L43). HTTP request bodies are capped at 1 MiB.
- Claude Code, Codex and Cursor plugins with a SessionStart hook.
- The Agent Pager adds subscribable room resources (README L72).
- Concurrency model: a single server process owns the store **[Inf]**; multi-process access is not documented **[?]**.

**Benchmarks.** "M7 deferred; docs make no benchmark-backed … claims" **[V]** (L35).

**Verdict: borrow ideas.** It is the best reference design for coding-agent memory: immutable episodes plus derived facts, namespaces, work items and handoffs, recall returned as untrusted text, an "honest scope" document, and no LLM on writes. Optionally run it as a prototype comparison. **Avoid it as a dependency**: single maintainer, dormant four months, niche engine, no compaction.

### 3.3 Memorose (ai-akashic/Memorose)

**Identity and maturity [V].** Apache-2.0, created 2026-02-08, last commit 2026-06-07 (`5be99b91`). 2 contributors, 31 stars, no GitHub releases, 0 commits in 90 days. The SDK repo calls it "CortexDB / Memorose". There are several unrelated "memorose" repos, but `ai-akashic/Memorose` is the canonical one.

**What it is.** A self-hosted Rust server (Axum) with a Next.js dashboard. It combines RocksDB (key-value), LanceDB (vectors), Tantivy (BM25), petgraph and openraft 0.9 **[V]** (memorose-core Cargo.toml).

**Data model.**
- L0 events → L1 facts and procedures → L2 insights (Louvain/LPA communities plus LLM synthesis) → L3 goals **[C]**.
- Scopes: agent, user, organisation **[C]** ([README L44-L46](https://github.com/ai-akashic/Memorose/blob/5be99b917bd3a15b7236446cfc850a53381a1e60/README.md#L44-L46)).
- Contradictions go through an LLM "Arbitrator" that returns OBSOLETE/CONTRADICTS/REAFFIRM/IGNORE actions, followed by logical tombstoning and `EvolvedTo`/`Contradicts` relations **[C/V]** ([TODO.md L28-L36](https://github.com/ai-akashic/Memorose/blob/5be99b917bd3a15b7236446cfc850a53381a1e60/TODO.md#L28-L36)).
- Multimodal input (text, image, audio, video) **[C]**.

**Write path.** LLM-dependent (Gemini or OpenAI clients in `llm/`): extraction, arbitration, insight synthesis **[V]**. Calls per write are not documented (at least one; more with arbitration) **[Inf]**.

**Retrieval.** Vector plus full-text plus graph expansion plus reranking. `/v1/memory/context` returns prompt-ready context within a token budget **[V/C]** (README L166, L449).

**Durability [V].** RocksDB opened with `Options::default()` ("defaults are fine for now"), and `put` uses default write options, so writes are logged but not fsynced ([kv.rs L13-L21](https://github.com/ai-akashic/Memorose/blob/5be99b917bd3a15b7236446cfc850a53381a1e60/crates/memorose-core/src/storage/kv.rs#L13-L21)). Raft votes and log entries go through the same store (`raft/storage.rs`). **[Inf]** The claimed Raft replication is therefore not power-loss safe.

**Interfaces.** HTTP only. MCP was rejected on purpose ("extra RTT, wasted tokens"), and no MCP code was found **[V]** ([TODO L6](https://github.com/ai-akashic/Memorose/blob/5be99b917bd3a15b7236446cfc850a53381a1e60/TODO.md#L6)).

**Benchmarks.** None: "publish reproducible workloads before treating numbers as release claims" **[V]** (README L384).

**Verdict: avoid.** Borrow only the token-budgeted context endpoint and the "context sidecar vs tool call" idea.

### 3.4 Statelet (stateletlab)

**Identity [V].**
- The engine repo `stateletlab/statelet` returns 404 (private).
- Public pieces: `statelet-sdk` (Apache-2.0 clients wrapping one gRPC proto), `statelet-longmemeval` (the benchmark harness, which also hosts the release binaries), `homebrew-statelet`, `stateletWeb`.
- Versions: v0.1.7 / lite-v0.1.7, released 2026-09-10. Nothing pushed since 2026-09-14.
- The site says "The source release is coming soon" ([statelet.ai](https://statelet.ai)).
- Licence conflict: the site says FSL-1.1-ALv2 **[C]**; the Homebrew formula for the full `statelet` says Apache-2.0 and `statelet-lite` says FSL **[V]** ([statelet.rb L2-L5](https://github.com/stateletlab/homebrew-statelet/blob/67b4ab8fdd3e493c0e441941e76848f01f8c99da/Formula/statelet.rb#L2-L5)).
- Unrelated "statelet" repos exist (JS/Java state machines).

**Claims [C]** (statelet.ai metadata): LSM-tree with MVCC, Raft per shard plus a metadata Raft group, DiskHNSW/SPFresh vector search, an openCypher-style memory query language with AS OF, temporal supersede chains, a `SubscribeCommitted` CDC feed, Redis RESP2 compatibility, RBAC, ONNX embeddings, an npm memory plugin for coding agents.

**What the public proto shows [V]** ([statelet.proto](https://github.com/stateletlab/statelet-sdk/blob/f2f64cb093c77b234aef47243caf506338b83eed/proto/statelet.proto#L183-L540)):
- `AgentSupersedeFact` (L349), `AgentMemoryIngest` (L356), `AgentQueryProvenance` (L366), `AgentManageTeamGrant` (L373);
- run checkpointing (`AgentStartRun`, L381+), `AgentBeliefQuery` (L414);
- streaming `SubscribeCommitted` (L532);
- `AgentFork` / merge-branch (L535+), `HybridSearch` (L183).

This proves an API surface, not behaviour.

**Write path [V].** In the benchmark, ingest runs inside the gateway, and LLM extraction is optional via `STATELET_LLM_EXTRACT=1` ([README L192, L210](https://github.com/stateletlab/statelet-longmemeval/blob/7d1bcece73231ef6605f380f2bbf9c4f3751b53b/README.md#L188-L212)).

**Benchmarks [C/V].** 91.60% (458/500) on LongMemEval_S, following mem0's methodology, with DeepSeek as reader and judge. The harness is public, but the ingest configuration "cannot [be] set, read back, or recorded" by the harness (L13, L196-L198). Not independent.

**Verdict: avoid for now; watch.** Borrow API ideas: a durable commit-feed CDC stream, a superseding-fact RPC, provenance-chain queries, team grants, run checkpoints and forks. Re-evaluate if the source is released.

### 3.5 HelixDB: agent-memory positioning and MCP only

**[V]** Apache-2.0, very active (38 contributors, 1,657 commits since 2026-08-12, v3.6.2 released 2026-11-10).

**Positioning.**
- The FAQ says its use is "long-term agent memory, GraphRAG…" ([README L318](https://github.com/HelixDB/helix-db/blob/b524727a283f0197ae64249bad51b80bf607a371/README.md#L316-L320)).
- Its long-term-memory guide assigns Extractor, Embedder, Adjudicator, Summarizer and Sweeper to **the application**, and only the store to the database ([guide L39-L47](https://github.com/HelixDB/helix-db/blob/b524727a283f0197ae64249bad51b80bf607a371/docs/learn/ai-memory/long-term-memory-for-ai-agents.mdx#L39-L47)).

**MCP.**
- `helix chef` installs a *docs* MCP and query skills for coding agents (README L63).
- The data MCP is **Helix Cloud only**: `https://mcp.helix-db.com/mcp` with WorkOS OAuth or service bearer tokens. Tools: `helix_execute_read_query`, plus `helix_prepare_write_query` → `helix_execute_write_query` with a five-minute one-time confirmation ([mcp.mdx L10-L76](https://github.com/HelixDB/helix-db/blob/b524727a283f0197ae64249bad51b80bf607a371/docs/database/helix-cloud/connect/mcp.mdx#L10-L76)).
- No MCP server module was found in the open-source crates **[V]** (tree search).

**Verdict: storage candidate only** (internals are covered by another thread). Borrow the two-step confirm for MCP writes and the "returned fields are untrusted" posture.

### 3.6 memvid v2 (memvid/memvid, crate `memvid-core`)

**Maturity [V].** Apache-2.0, 17.4k stars, 24 contributors, 42 open issues. v2.0.140 released 2026-05-27; last commit 2026-07-14; 0 commits since 2026-08-12. Not to be confused with the original Python QR-in-video memvid or `AllenDang/memvid-rs`.

**What it is.** An embedded single-file (`.mv2`) store made of append-only "Smart Frames", with Tantivy BM25, HNSW plus local ONNX embeddings, optional CLIP/Whisper, and encryption **[V]**.
- Memory cards: `MemoryKind` (fact, preference, event, profile, relationship, goal), entity/slot/value, `VersionRelation` (sets, updates, extends, retracts), `event_date`/`document_date`, `source_frame_id`/`source_uri`, `confidence` ([memory_card.rs L19-L236](https://github.com/memvid/memvid/blob/e6bd9f7b9c38cd8d5370fa0fc936ac1dcd751813/src/types/memory_card.rs#L19-L236)).
- ACL filtering by tenant, principal, group and role **[V]** (`src/memvid/acl.rs`).

**Durability [V].**
- An embedded WAL with blake3-checksummed records, `sync_all()` after each append unless batch mode is on ([wal.rs L227-L275](https://github.com/memvid/memvid/blob/e6bd9f7b9c38cd8d5370fa0fc936ac1dcd751813/src/io/wal.rs#L227-L275)).
- OS-level exclusive writer lock and shared reader locks via `fs2` ([lock.rs L24-L55](https://github.com/memvid/memvid/blob/e6bd9f7b9c38cd8d5370fa0fc936ac1dcd751813/src/lock.rs#L24-L55)).
- Bugs: #230 ("Embedded WAL is corrupted… checksum mismatch on 16th put", closed 2026-05-27). The related WAL fix #231 and batch-replay fix #249 are still **open**. #260 (19 RustSec advisories in compression dependencies) is open.
- Concurrent writers are an open request (#218).

**Interfaces.** Rust, Node, Python, CLI. No MCP in core; there are community MCP wrappers in Python **[V]**.

**Benchmarks.** "+35% SOTA on LoCoMo", "0.025 ms P50" **[C]** ([README L45-L53](https://github.com/memvid/memvid/blob/e6bd9f7b9c38cd8d5370fa0fc936ac1dcd751813/README.md#L45-L53)). The harness is `memvid/memvidbench` (6 stars). Not independent.

**Verdict: borrow ideas** (checksummed embedded WAL, the memory-card versioning vocabulary, the portable capsule file). **Avoid as a shared-service substrate** (single writer, WAL fixes unmerged, development paused).

### 3.7 cortex-mem (sopaco/cortex-mem)

**Maturity [V].** MIT, 320 stars, 5 contributors. Release 2.7.1 on 2026-04-25; last commit 2026-07-22.

**What it is.** A thin wrapper **[V]**:
- markdown files under a `cortex://` virtual filesystem;
- an **external Qdrant** for vectors;
- LLM extraction and generation of L0 (abstract) / L1 (overview) / L2 (detail) layers;
- Ebbinghaus-curve cleanup;
- tenant-aware collection naming ([README L101-L116](https://github.com/sopaco/cortex-mem/blob/82a1c83c4b1c2d03d69d825309040f370dc16635/README.md#L101-L116)).

MCP is stdio only (`rmcp::transport::stdio`) ([main.rs L6, L151](https://github.com/sopaco/cortex-mem/blob/82a1c83c4b1c2d03d69d825309040f370dc16635/cortex-mem-mcp/src/main.rs#L151)).

**Benchmarks.** LoCoMo "conv-26, 152 questions" (one of ten conversations) with an LLM judge **[V/C]** (L377). "80%+ token savings" vs OpenClaw's built-in memory **[C]** (L281).

**Verdict: avoid.** Borrow the progressive-disclosure idea (L0/L1/L2 summaries to save tokens).

### 3.8 shodh-memory (varun29ankuS/shodh-memory)

**Maturity [V].** Apache-2.0, 323 stars, 12 contributors, 33 open issues. Tags up to v0.2.0. 150 commits since 2026-08-12; last main commit 2026-09-27.

**What it is.** A single binary (about 17 MB) with a server (160+ REST endpoints), TUI, Python and Rust APIs, and 51 MCP tools including todos, projects, reminders and backups **[C/V]** ([README L194-L242](https://github.com/varun29ankuS/shodh-memory/blob/902daf8394e74c93056be34fe5fdc5bb4f6bfbec/README.md#L194-L242)).
- **No LLM calls** on write or recall. It runs these models locally: MiniLM embeddings, an ms-marco cross-encoder reranker, and GLiNER named-entity recognition, all via ONNX; relation extraction is rule-based **[C/V]** (L36, L47-L55).
- Hebbian strengthening, decay, spreading activation; working → session → long-term memory tiers (RocksDB) **[C]**.
- Content-hash deduplication **[C]** (L190).

**Durability [V].** RocksDB. **Writes default to async (no fsync)**: "survives process crashes but NOT power loss"; override with `SHODH_WRITE_MODE=sync` ([storage.rs L41-L56](https://github.com/varun29ankuS/shodh-memory/blob/902daf8394e74c93056be34fe5fdc5bb4f6bfbec/src/memory/storage.rs#L41-L56)). Issue #391, "LongTerm compression discards memory body … permanent data loss", was closed (fixed).

**Interfaces [V].** Rust MCP over stdio (`rmcp` with `transport-io`), plus an npm MCP proxy to the HTTP server. Multi-user via `user_id`; API key auth. Zenoh/ROS2 transport.

**Benchmarks.** +16.5 percentage points precision@1 on LoCoMo held-out questions from reranking; latency tables measured on CI runners **[C]**. Not independent.

**Verdict: borrow ideas, and use as a baseline.** Useful as a cheap, deterministic, LLM-free comparison point. Not a multi-agent shared service with provenance or two time axes **[Inf]**.

### 3.9 huggingface/funes

**Maturity [V].** Apache-2.0, 592 stars, 12 contributors, under the Hugging Face organisation. v1.12.0 released 2026-11-10; actively maintained.

**What it is.** An index of past agent sessions (Claude Code, Codex, pi, Hermes) **[V]**:
- parse → chunk → embed into a local **Lance** dataset;
- recall fuses vector and BM25 search, then reranks with a cross-encoder;
- **no generative model**; exact source references (`session_id` + `turn_uuid`) ([README L150](https://github.com/huggingface/funes/blob/7250df3a7daa3b2aa8629cd8923fd5624a49163b/README.md#L146-L152); [RATIONALE L21-L45](https://github.com/huggingface/funes/blob/7250df3a7daa3b2aa8629cd8923fd5624a49163b/docs/RATIONALE.md#L21-L45)).

**Design choice.** An append-only log with no write-time supersession: obsolete information is judged at read time ("the superseded passage is often the answer itself"). Recall is pulled by the agent, never pushed into context (L84).

**Sharing [V].** `funes push` publishes to a Hugging Face Hub dataset, private by default, behind a secrets gate (TruffleHog). Remote recall reads immutable Lance files over `hf://` with a cache pinned to a commit.

**Interfaces [V].** CLI; read-only stdio MCP (`funes mcp`); hooks index every turn ([add.md L176-L212](https://github.com/huggingface/funes/blob/7250df3a7daa3b2aa8629cd8923fd5624a49163b/docs/add.md#L176-L212)).

**Limits [Inf].** No structured facts or entities, no write API for agents, and sharing is snapshot publishing rather than concurrent live writes.

**Verdict: use as-is as the episodic baseline** and possibly as hivemind's raw-transcript layer. Borrow its principles: deterministic ingest, exact source references, read-time resolution.

### 3.10 YantrikDB (yantrikos/yantrikdb, -server)

**[V]** Apache-2.0, 11 contributors, v0.23.4 released 2026-10-28, 32 open issues. An embedded Rust engine on **bundled SQLite** with Python bindings: decay, consolidation, contradiction detection (`think()`), and a knowledge graph ([README L1-L18](https://github.com/yantrikos/yantrikdb/blob/bbb5f52797b0bfb7c85c5e9fae65eb0a53ff2d8b/README.md#L1-L18)).

- MCP is via the Python `yantrikdb-mcp` package.
- Explicit warning: a second SQLite connection in the same process "corrupts the store silently" (L52-L60).
- The server repo claims multi-node HA **[C]**.

**Verdict: borrow ideas** (contradiction and decay heuristics, embedding-model choice measured on real agent memory). Not a core candidate.

### 3.11 Others found and set aside (breadth)

Metadata **[V]**; capability descriptions are **[C]** from READMEs.

| Repo | Why not in depth |
|---|---|
| codecoradev/uteke (Apache-2.0, 3 contributors) | One-binary MCP memory on SQLite FTS plus local vectors; claims 98.2% recall@5 on LongMemEval-S with raw outputs committed (retrieval, not QA). Small team, has a cloud upsell. Worth reproducing as a baseline. |
| nambok/mentedb (3 contributors) | Self-built engine (WAL, HNSW, knowledge graph) plus cloud; LLM-backed nightly maintenance. Young. |
| topoteretes/cognee-rs (11 contributors) | Rust port of the Python cognee: SQLite + Ladybug graph + LanceDB, LLM required. v0.2.0; 44 open issues. |
| ourmem/omem (licence NOASSERTION) | Shared "Spaces" (personal/team/org); last push 2026-05-24. Unclear licence. |
| hsuanshao/TeamAgentMemory | Team memory with L0→L3 and asynchronous LLM; created 2026-10-18. Too new. |
| LamantinAI/kaeru (licence NOASSERTION) | Team-shareable via cloud; unclear licence. |
| JordyZomer/lemmalog (MIT, 5 contributors) | Datalog memory with two time axes, confidence × provenance annotations, `why()` proof trees, MCP. An idea source for derivation provenance. |
| xerj-org/xerj (100 contributors) | An AI search engine/MCP over stored facts (UNKNOWN/CONFLICT answers). Search, not agent memory; a potential adjacent component. |
| ruvnet/RuVector (MIT, 4.7k stars, 273 open issues) | Vector/GNN database marketed for agent memory; broad claims; out of scope here. |
| iwe-org/iwe | Markdown knowledge-graph LSP plus MCP; a note tool, not a shared memory service. |
| Graphicle-AI/graphicle | Code graph plus memory; created 2026-10-14. |
| lovstudio/Ataru, ZhangHanDong/mempal, KirtiRamchandani/memory.cpp, linggen/linggen-memory | Local single-user agent-history search; 1–5 contributors. |
| cyberlife-coder/VelesDB, NodeDB-Lab/nodedb, context-graph-ai/contextdb, anaslimem/CortexaDB, shuruheel/mnestic | Database primitives ("memory engines"); belong to the storage thread. Several have unclear licences or one contributor. |
| alphaonedev/ai-memory-mcp | 1 contributor with **1,653 open issues**: an anomaly / automation red flag. |
| rand/mnemosyne | Inactive since 2025-11. |
| Dakera-AI | Rust SDK/MCP; claims 88.2% on LoCoMo; core engine not inspected **[?]**. |

---

## 4. Comparison tables

### 4.1 Architecture

| System | Kind | Language | Licence | Deployment | Storage engine | Search modes | Time / provenance | Concurrency | MCP / RPC |
|---|---|---|---|---|---|---|---|---|---|
| AndaDB + Nexus | DB primitive + memory engine | Rust | MIT | Embedded; HTTP/JSON-RPC server; shard proxy; Durable Objects | Own doc store on `object_store` (FS/S3) | B-Tree, BM25, HNSW, RRF (core); Nexus: keyword + graph only | Two time axes (FOR TIME / AS OF SEQ), Assertion/Evidence/Activity, supersession **[V]** | Single writer per DB; in-process RwLock | No MCP; HTTP JSON-RPC |
| Anda Brain | Memory service (LLM agents) | Rust | Apache-2.0 | Server; Cloudflare worker | Nexus/AndaDB | LLM-planned recall over graph | Inherits KIP | Per-Space DB | MCP stdio + Streamable HTTP **[V]**; REST |
| Aionforge | Memory substrate | Rust | Apache/MIT | Embedded crate; single binary; Docker | selene-db (WAL, group-commit fsync) | BM25 + vector + graph + recency/trust | Edge-level two time axes; SUPERSEDED_BY/CONTRADICTS; provenance nodes; no file pointers | Single process **[Inf]** | MCP stdio + Streamable HTTP **[V]** |
| Memorose | Memory runtime | Rust | Apache-2.0 | Server + dashboard; Raft **[C]** | RocksDB + LanceDB + Tantivy | Vector + full-text + graph + rerank | Tombstone + EvolvedTo/Contradicts via LLM | Raft **[C]**; no fsync **[V]** | REST only |
| Statelet | Distributed DB + memory APIs | Rust **[C]** | FSL/Apache (conflicting) | Cluster / lite binary | LSM + Raft **[C]** | Vector (DiskHNSW/SPFresh), hybrid, graph **[C]** | Supersede chains, AS OF, provenance RPCs **[C/V proto]** | Raft, MVCC **[C]** | gRPC **[V]**; Redis RESP **[C]**; MCP **[?]** |
| HelixDB | Graph/vector DB | Rust | Apache-2.0 | Embedded; server; Cloud | Own engine on object storage | Graph + vector + BM25 | Application-defined | Single writer, scaling readers (Cloud) **[C]** | Cloud MCP (OAuth) **[V]**; HTTP |
| memvid v2 | Embedded file format | Rust | Apache-2.0 | Embedded; CLI | Single `.mv2` file, embedded WAL | BM25 + HNSW + CLIP | Memory cards with VersionRelation, source_frame_id | One exclusive writer (fs2) **[V]** | None in core |
| cortex-mem | Wrapper | Rust | MIT | Service + MCP + CLI | Markdown FS + **Qdrant** | Vector (weighted L0/L1/L2) | Minimal | Multi-tenant collections | MCP stdio |
| shodh-memory | Memory server | Rust | Apache-2.0 | Single binary; Docker; Python | RocksDB (async by default) | Vector + BM25 + cross-encoder + graph | Causal edges, decay; no two time axes **[?]** | Server; per-user | MCP stdio (Rust + npm); REST |
| funes | Transcript index | Rust | Apache-2.0 | CLI + hooks; HF Hub | Lance | Vector + BM25 + cross-encoder | Exact session/turn refs; append-only | Local writer; publish snapshots | MCP stdio (read-only) |
| YantrikDB | Embedded engine | Rust (+Py) | Apache-2.0 | Embedded; server | SQLite | Hybrid lexical + vector | Contradiction flags, decay | SQLite single writer; caveats **[V]** | MCP via Python |

### 4.2 Writes, evidence and maturity

| System | LLM on write | Benchmarks (who) | Last commit | Contributors | Verdict |
|---|---|---|---|---|---|
| AndaDB / Nexus | No (Nexus); Brain: yes (agentic) | Engine microbenchmarks (vendor); no quality benchmark | 2026-10-26 | 3 / 1 | Borrow ideas |
| Aionforge | **No** | None (deferred) | 2026-07-11 | 1 | Borrow ideas / prototype comparison |
| Memorose | Yes | None | 2026-06-07 | 2 | Avoid |
| Statelet | Optional | LongMemEval-S 91.6% (vendor) | n/a (private) | ? | Avoid now; watch |
| HelixDB | n/a (application) | n/a | 2026-11-10 | 38 | Storage candidate (other thread) |
| memvid | Optional enrichment | LoCoMo (vendor) | 2026-07-14 | 24 | Borrow ideas |
| cortex-mem | Yes | LoCoMo, 1 conversation (vendor) | 2026-07-22 | 5 | Avoid |
| shodh-memory | **No** | LoCoMo retrieval gates (vendor) | 2026-09-27 (main) | 12 | Baseline / borrow |
| funes | **No** | None published **[?]** | 2026-11-10 | 12 | Use as-is (episodic baseline) |
| YantrikDB | No (heuristic) | Small internal MRR study (vendor) | 2026-11-02 | 11 | Borrow ideas |

---

## 5. Implications for hivemind

**Adopt or use**
- **funes** as the baseline (and optionally the component) for raw session memory: deterministic, no LLM, exact source references, shareable as an artifact. Every hivemind feature should beat "funes + agent reads passages" on outcomes per token **[Inf]**.
- **MCP over Streamable HTTP and stdio via `rmcp`.** Aionforge, Anda Brain and shodh all use it **[V]**, so it is the de facto Rust path.

**Borrow ideas**
- From **KIP**: separate Proposition, Assertion and Evidence; keep valid time apart from record time; use three distinct correction operations (succession, supersession, recording repair); keep governance out of reach of memory content (prompt-injection resistance); compute strength, confidence and trust separately; never store belief, compute it; use export/import capsules.
- From **Aionforge**: immutable episodes with derived facts beside them; agent/team/global namespaces; WorkItems and handoffs as first-class records outside decay; return recall as a bounded bundle marked untrusted; document scope honestly. Fix its gap by adding first-class file/commit/URI source pointers, which coding agents need.
- From **memvid**: checksummed WAL records with fsync by default, and a portable single-file capsule for offline handoff.
- From **Statelet's proto**: a CDC feed of committed changes for notifications, run checkpoints and forks, team grants.
- From **HelixDB**: a prepare/confirm pair for MCP writes.
- From **cortex-mem / Memorose**: token-budgeted, progressive (L0/L1/L2) context.

**Avoid**
- Write paths that depend on LLM extraction by default (Memorose, cortex-mem, Anda Brain). They cut against the compute and token goal, and they break the link to the source.
- Default-async durability (shodh, Memorose) for a shared hippocampus.
- Depending on single-maintainer engines (selene-db) or closed engines (Statelet).

**Build stance [Inf].** A small hivemind core on a proven embedded storage substrate, chosen by the other threads:
- append-only episodes;
- typed assertions with two time axes, supersession links and source pointers;
- scoped namespaces;
- hybrid BM25 + vector search with an optional reranker;
- a CDC/subscribe stream;
- MCP over stdio and Streamable HTTP, plus optional gRPC.

**What would change this conclusion**
- Aionforge resumes, gains maintainers, and moves to a widely used storage engine. It would then be a strong extend-or-fork candidate.
- Statelet releases its engine and a third party reproduces its LongMemEval result.
- AndaDB adds safe multi-writer access, or the Nexus adds semantic search, with more contributors.
- An independent benchmark shows an LLM-extraction system beating funes/shodh-style raw recall per token on coding-agent tasks.

---

## 6. Open questions

1. Statelet: engine language and code quality, its real MCP support, and the licence conflict (Apache-2.0 vs FSL) — all unverifiable while the source is private.
2. Anda Brain: LLM calls and tokens per Formation and per Maintenance cycle; whether its MIB benchmark harness has published results.
3. Aionforge: is the project paused or abandoned? Is selene-db 1.3's WAL sync policy the same as v1.4.0's? How does it behave with several processes?
4. memvid: why has development stopped since July 2026? Will the WAL fixes (#231, #249) merge?
5. HelixDB: is there an MCP surface for self-hosted v3?
6. No independent LoCoMo/LongMemEval evaluations of any candidate were found. Uteke's and shodh's committed raw outputs could be re-run.
7. The issue-search sweep was rate-limited for funes, YantrikDB and Aionforge **[?]**.

---

## 7. Sources (all checked 2026-11-10)

1. https://github.com/ldclabs/anda-db/blob/8ad402736559eba06b81a71f23612d0eff0fd64b/README.md
2. https://github.com/ldclabs/anda-db/blob/8ad402736559eba06b81a71f23612d0eff0fd64b/docs/anda_db.md#L428-L549
3. https://github.com/ldclabs/anda-db/blob/8ad402736559eba06b81a71f23612d0eff0fd64b/docs/anda_cognitive_nexus.md
4. https://github.com/ldclabs/anda-db/blob/8ad402736559eba06b81a71f23612d0eff0fd64b/rs/anda_db/tests/crash_recovery.rs#L1-L16
5. https://github.com/ldclabs/anda-db/blob/8ad402736559eba06b81a71f23612d0eff0fd64b/rs/anda_cognitive_nexus_server/README.md
6. https://github.com/ldclabs/anda-db/blob/8ad402736559eba06b81a71f23612d0eff0fd64b/docs/benchmarks/anda_query_scale_2026-09-26-rerun/README.md
7. https://github.com/ldclabs/KIP/blob/c4f3c17a1306d89562f3778af0013f0de17436a0/README.md
8. https://github.com/ldclabs/anda/blob/b42da9388048b89cb50e04a4c04ac2952cdada43/README.md
9. https://github.com/ldclabs/anda-brain/blob/4d4d7faa3bc1947bdc55920d64a4d497377607c2/README.md
10. https://github.com/ldclabs/anda-brain/blob/4d4d7faa3bc1947bdc55920d64a4d497377607c2/anda_brain/src/mcp.rs
11. https://github.com/jscott3201/aionforge-memory/blob/cd5aa35512ef171ddf09b89426c69f13351b5ccf/README.md
12. https://github.com/jscott3201/aionforge-memory/blob/cd5aa35512ef171ddf09b89426c69f13351b5ccf/docs/data-model.md
13. https://github.com/jscott3201/aionforge-memory/blob/cd5aa35512ef171ddf09b89426c69f13351b5ccf/docs/honest-scope.md
14. https://github.com/jscott3201/aionforge-memory/blob/cd5aa35512ef171ddf09b89426c69f13351b5ccf/docs/operations-recovery.md#L78-L160
15. https://github.com/jscott3201/aionforge-memory/blob/cd5aa35512ef171ddf09b89426c69f13351b5ccf/Benchmarks.md
16. https://github.com/jscott3201/selene-db/blob/v1.4.0/crates/selene-persist/src/writer.rs#L27-L51
17. https://github.com/ai-akashic/Memorose/blob/5be99b917bd3a15b7236446cfc850a53381a1e60/README.md
18. https://github.com/ai-akashic/Memorose/blob/5be99b917bd3a15b7236446cfc850a53381a1e60/TODO.md
19. https://github.com/ai-akashic/Memorose/blob/5be99b917bd3a15b7236446cfc850a53381a1e60/crates/memorose-core/src/storage/kv.rs#L13-L21
20. https://github.com/ai-akashic/Memorose/blob/5be99b917bd3a15b7236446cfc850a53381a1e60/crates/memorose-core/src/raft/storage.rs
21. https://statelet.ai (site metadata and FAQ)
22. https://github.com/stateletlab/statelet-longmemeval/blob/7d1bcece73231ef6605f380f2bbf9c4f3751b53b/README.md
23. https://github.com/stateletlab/statelet-sdk/blob/f2f64cb093c77b234aef47243caf506338b83eed/proto/statelet.proto
24. https://github.com/stateletlab/homebrew-statelet/blob/67b4ab8fdd3e493c0e441941e76848f01f8c99da/Formula/statelet.rb
25. https://github.com/HelixDB/helix-db/blob/b524727a283f0197ae64249bad51b80bf607a371/README.md
26. https://github.com/HelixDB/helix-db/blob/b524727a283f0197ae64249bad51b80bf607a371/docs/database/helix-cloud/connect/mcp.mdx
27. https://github.com/HelixDB/helix-db/blob/b524727a283f0197ae64249bad51b80bf607a371/docs/learn/ai-memory/long-term-memory-for-ai-agents.mdx
28. https://github.com/memvid/memvid/blob/e6bd9f7b9c38cd8d5370fa0fc936ac1dcd751813/README.md
29. https://github.com/memvid/memvid/blob/e6bd9f7b9c38cd8d5370fa0fc936ac1dcd751813/src/io/wal.rs
30. https://github.com/memvid/memvid/blob/e6bd9f7b9c38cd8d5370fa0fc936ac1dcd751813/src/lock.rs
31. https://github.com/memvid/memvid/blob/e6bd9f7b9c38cd8d5370fa0fc936ac1dcd751813/src/types/memory_card.rs
32. https://github.com/memvid/memvid/issues/230 , /issues/218 , /pull/231 , /pull/249 , /issues/260
33. https://github.com/sopaco/cortex-mem/blob/82a1c83c4b1c2d03d69d825309040f370dc16635/README.md
34. https://github.com/sopaco/cortex-mem/blob/82a1c83c4b1c2d03d69d825309040f370dc16635/cortex-mem-mcp/src/main.rs
35. https://github.com/varun29ankuS/shodh-memory/blob/902daf8394e74c93056be34fe5fdc5bb4f6bfbec/README.md
36. https://github.com/varun29ankuS/shodh-memory/blob/902daf8394e74c93056be34fe5fdc5bb4f6bfbec/src/memory/storage.rs#L41-L56
37. https://github.com/varun29ankuS/shodh-memory/issues/391
38. https://github.com/huggingface/funes/blob/7250df3a7daa3b2aa8629cd8923fd5624a49163b/README.md
39. https://github.com/huggingface/funes/blob/7250df3a7daa3b2aa8629cd8923fd5624a49163b/docs/RATIONALE.md
40. https://github.com/huggingface/funes/blob/7250df3a7daa3b2aa8629cd8923fd5624a49163b/docs/add.md
41. https://github.com/yantrikos/yantrikdb/blob/bbb5f52797b0bfb7c85c5e9fae65eb0a53ff2d8b/README.md
42. Repo metadata (licence, dates, contributors, releases) via `gh api repos/<owner>/<repo>` for every repo named above, including codecoradev/uteke, nambok/mentedb, topoteretes/cognee-rs, ourmem/omem, JordyZomer/lemmalog, xerj-org/xerj, ruvnet/RuVector, alphaonedev/ai-memory-mcp.
