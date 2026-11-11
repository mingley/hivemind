# 02: Landscape: what exists, what it does, what we take

Status: v1.0 (2026-11-10). This synthesizes nine research threads in
[`research/`](../research/README.md), all checked against public primary
sources on 2026-11-10. Labels follow
[01](01-problem-and-goals.md#evidence-labels). Each claim here links to the
report that holds its primary sources.

## Bottom line

1. **No existing system covers the brief.** None offers a lean,
   multi-agent, provenance-carrying, version-aware memory with MCP and a
   service mode. The best *semantics* sit in small or dormant Rust projects;
   the mature frameworks are Python services with LLM-heavy writes and
   external databases
   ([research/01](../research/01-agent-memory-systems-rust.md),
   [research/03](../research/03-memory-frameworks.md)).
2. **Vendor benchmark numbers are unreliable.** Every LoCoMo and LongMemEval
   figure for the Rust candidates is self-reported. The Mem0 versus Zep
   dispute included an arithmetic error. In Mem0's own paper, passing the
   full conversation (72.9%) beats every memory system
   ([research/03](../research/03-memory-frameworks.md#24-benchmark-disputes-and-replications)).
3. **Independent coding-agent evidence is sobering** **[I]**
   ([research/04](../research/04-coding-agent-memory-practice.md#7-pain-points-and-failure-modes-of-todays-approaches),
   [research/03](../research/03-memory-frameworks.md)):
   - Memory systems failed to beat no memory in 11 of 12 pairings
     (VibeMemBench).
   - 29 systems scored 46–53%, against 47.8% with no memory and 50.0% for a
     plain full-text-search baseline (MemDev-Bench).
   - Always-loaded context files add more than 20% to cost without a general
     gain.
   - Agents followed a stale instruction file over the code in 90.1% of
     choices.
4. **What does help:** *verified* experience delivered as *evidence*;
   retrieval quality more than write-time extraction; explicit supersession
   rather than LLM-judged invalidation; code citations checked at read time
   ([research/03](../research/03-memory-frameworks.md#5-implications-for-hivemind),
   [research/04](../research/04-coding-agent-memory-practice.md#8-patterns-hivemind-should-adopt-or-avoid-inf-grounded-in-cited-evidence),
   [research/05](../research/05-research-and-benchmarks.md#4-design-constraints-from-evidence)).
5. **Product memory has converged on one shape:** a small always-loaded index
   plus detail files, written by the agent. It is siloed per vendor and
   machine, and changes often. Cursor, Windsurf, Gemini CLI, and Devin
   Knowledge have all removed or replaced memory features
   ([research/04](../research/04-coding-agent-memory-practice.md#1-summary)).

These findings shaped the decisions in [04](04-architecture-and-decisions.md).
The owner's call to build one integrated, all-Rust system (D1, D2) rests on
the first point. The evaluation's insistence on strong simple baselines
([05](05-evaluation-plan.md)) rests on points 2–4.

## Agent-memory systems

The table separates claimed, implemented, and demonstrated capabilities. "Demo"
means independent evidence; none of these systems has independent evidence on
coding tasks.

| System | What it actually is | Stack and license | Time, provenance, supersession | Write path | MCP | Maturity (2026-11-10) | Verdict for hivemind |
|---|---|---|---|---|---|---|---|
| **Aionforge Memory** | Memory substrate: immutable episodes, derived facts, namespaces, work items, agent pager | Rust, Apache-2.0/MIT; its author's own graph engine | `SUPERSEDED_BY`, `CONTRADICTS`, two time axes on edges **[V]**; no file or commit pointers | No LLM **[V]** | stdio, Streamable HTTP **[V]** | One maintainer; no commits since 2026-07-11 | Closest semantics; **borrow ideas**, don't depend on it |
| **KIP / Cognitive Nexus on AndaDB** | Protocol plus engine with the most rigorous belief model | Rust, MIT | Proposition separate from assertion; `FOR TIME` vs `AS OF SEQ`; three correction kinds; governance out of reach of content **[V]** | Engine has no LLM; Anda Brain uses LLM agents | Anda Brain only | 1–3 contributors; single writer; no semantic search in the engine | **Borrow the model** ([03](03-memory-model.md#lifecycle-operations)) |
| **Statelet** | Claims a distributed temporal memory database | Engine source not public; license signals conflict | Claimed; the proto shows supersede and provenance RPCs **[V]** | Optional LLM | ? | Private engine | Watch; borrow API ideas (CDC feed, forks) |
| **Memorose** | LLM-heavy memory runtime | Rust, Apache-2.0 | LLM arbitration | LLM | None (on purpose) | Dormant since 2026-06; RocksDB writes not fsynced **[V]** | Avoid |
| **HelixDB** | Graph and vector database, not memory | Rust, Apache-2.0 | Application-defined | n/a | Cloud only | Very active | Storage reference only |
| **memvid v2** | Single-file embedded memory format | Rust, Apache-2.0 | Memory cards with version relations **[V]** | Optional | No (core) | Single writer; WAL fixes unmerged; quiet since 2026-07 | Borrow ideas (checksummed WAL, capsule file) |
| **shodh-memory** | No-LLM memory server | Rust, Apache-2.0 | Decay and causal edges | No LLM | stdio | Active; async writes by default **[V]** | Baseline; borrow ideas |
| **huggingface/funes** | Append-only index of past agent sessions | Rust, Apache-2.0, Lance | Exact `session_id` + `turn_uuid` references **[V]** | No LLM | Read-only stdio | Active; v1.12.0 on 2026-11-10 | **Episodic baseline to beat** |
| **Graphiti / Zep** | Temporal knowledge graph framework | Python, Apache-2.0, plus a graph DB | Bitemporal edges, episode provenance **[V]** | About 8–19 LLM calls per episode **[Inf]** | Yes | Active; in-memory MCP write queue (#1707) | Semantic reference; too heavy for the core |
| **Mem0 v2** | Vector memory with LLM extraction | Python, Apache-2.0 | ADD-only; contradictions accumulate (#4896) | 1 LLM call | Hosted only | Active | Not suitable for trustworthy coding memory |
| **Letta / Letta Code** | Agent framework; git-backed markdown memory | TypeScript and Python, Apache-2.0 | Git history | Agent's own tool calls | Client | Shared memory needs Letta Cloud | Adopt the git-projection *pattern* |
| **Basic Memory** | Markdown plus SQLite, local-first | Python, **AGPL-3.0** | Manual edits | No LLM | Yes | Single user | Fine for one developer today; not multi-agent |
| **Hindsight** | Memory service with history and sources | Python, MIT, embedded Postgres | Observations with history | LLM per retain | Per memory bank | Active | Best service to prototype *against* |
| **GitHub Copilot Memory** | Repository facts with code citations | Closed | Citations checked at use; 90-day expiry reset by upvote **[V]** | Agent tool call | n/a | Vendor A/B only: merge rate 90% vs 83% **[C]** | **Copy citation checks and expiry** |
| **Claude Code / Codex memory** | `MEMORY.md` index plus topic files | Vendor tools (Codex's pipeline is Rust) | Files and git | Agent; Codex extracts with an LLM | n/a | Machine-local | Treat as sources and sinks to sync with |
| **Beads / MCP Agent Mail** | Task ledger; mailboxes and file leases | Go/Dolt; Python/SQLite | Work items, claims | No LLM | Yes | Data-loss and corruption reports under concurrency | Copy the coordination primitives; avoid their storage choices |

Sources and details: [research/01](../research/01-agent-memory-systems-rust.md#4-comparison-tables),
[research/03](../research/03-memory-frameworks.md#4-tables),
[research/04](../research/04-coding-agent-memory-practice.md#6-comparison-table).

## Storage and retrieval foundations

hivemind builds its own engine (D2), so this survey informs *how* we build
rather than what we adopt
([research/02](../research/02-storage-and-retrieval-foundations.md),
[research/09](../research/09-engine-design-techniques.md)).

- **Durability defaults are often weaker than they look.**
  - Lance on local disk doesn't fsync by default.
  - Qdrant flushes its WAL about every 5 seconds unless the client asks to
    wait.
  - fjall and RocksDB don't sync each write by default.
  - SQLite's `synchronous=NORMAL` can lose recent commits, and it skips
    `F_FULLFSYNC` on macOS unless configured.

  Our engine fsyncs before every acknowledgement and tests it
  ([04](04-architecture-and-decisions.md#durability-contract-v0)).
- **Multi-process access is the norm-breaker.** One MCP stdio server per
  agent session means several processes per store. The official MCP memory
  server lost writes this way twice
  ([research/06](../research/06-architecture-distribution-protocols-security.md#42-mcp-stdio-means-many-processes-on-one-database)).
  Hence one daemon per store.
- **Exact vector search is feasible per scope** on a laptop: to about 100k
  vectors at f32, and about 1M with int8 or binary codes plus rescoring.
  Approximate indexes are needed only beyond that, or for server query rates
  ([research/02](../research/02-storage-and-retrieval-foundations.md#4-brute-force-knn-feasibility)).
- **Engine shape:** "Bitcask with segments":
  - a segmented append-only log;
  - a content-addressed blob store;
  - in-memory projections rebuilt by replay, then on-disk immutable index
    segments.

  This is the consensus of Kafka, Lucene, WiscKey, and TigerBeetle precedents
  ([research/09](../research/09-engine-design-techniques.md#1-summary-decisions-this-research-implies)).
- **No independent durability evidence** (no Jepsen analyses) exists for any
  embedded candidate. Our crash tests are our evidence.

## Research findings that constrain the design

| Finding | Evidence | Design consequence |
|---|---|---|
| Simple baselines (full context, files plus grep, SQLite FTS) match many memory systems | [research/03](../research/03-memory-frameworks.md), [research/05](../research/05-research-and-benchmarks.md#4-design-constraints-from-evidence) | The evaluation must include them ([05](05-evaluation-plan.md#baselines-and-conditions)) |
| Lossy LLM extraction at write time discards what retrieval later needs | [research/03](../research/03-memory-frameworks.md#5-implications-for-hivemind) | Keep raw evidence; agent-authored claims; no LLM on the hot path (D6) |
| A "newest statement wins" rule solves about 80% of conflict items; explicit supersession beats decay | [research/08](../research/08-computational-hippocampus.md#1-summary) | Supersede, invalidate, and retract are first-class ([03](03-memory-model.md#lifecycle-operations)) |
| Graph retrieval helps multi-hop questions and hurts simple facts | [research/05](../research/05-research-and-benchmarks.md), [research/08](../research/08-computational-hippocampus.md#41-mapping-table) | Graph spreading is an optional, tested mode (F10, C4) |
| Experience reuse helps when abstracted; raw traces can transfer negatively | [research/05](../research/05-research-and-benchmarks.md#26-experiential-and-procedural-memory-most-relevant-to-coding-agents) | Procedures with slots (v1); applicability conditions on everything |
| Memory poisoning works against production memory features | [research/06](../research/06-architecture-distribution-protocols-security.md#81-evidence-memory-poisoning-is-real), [research/05](../research/05-research-and-benchmarks.md#29-memory-security) | Trust tiers, taint, quarantine, and server-side ACLs from v0 |
| Instructions beat code when memory is delivered as instructions | [research/04](../research/04-coding-agent-memory-practice.md#7-pain-points-and-failure-modes-of-todays-approaches) | Deliver memory as evidence; current code wins (D7) |

## What we take from whom

| Idea | Source | Where in hivemind |
|---|---|---|
| Claim separate from the act of asserting it; "as of" versus "valid"; three kinds of correction; governance outside content | KIP / AndaDB | [03](03-memory-model.md#lifecycle-operations), [03](03-memory-model.md#provenance-verification-and-trust) |
| Immutable episodes plus derived facts; namespaces; work items; recall returned as untrusted text | Aionforge | Record types and scopes ([03](03-memory-model.md#record-types)) |
| Deterministic, LLM-free ingest; exact source references | funes, shodh-memory | D6; provenance |
| Bitemporal edges with episode provenance | Graphiti | Time model ([03](03-memory-model.md#time-bitemporal-plus-applicability-in-version-space)) |
| Citations checked at use; reinforce only on validated use; 90-day expiry | GitHub Copilot Memory | D7; belief policy |
| Tiny index with details on demand; agent-authored writes | Claude Code, Codex, Letta Code | Progressive disclosure; markdown export (M3) |
| Leases with time limits, mailboxes, ready queues | MCP Agent Mail, Beads | Coordination layer ([03](03-memory-model.md#coordination)) |
| Event log as truth; projections rebuilt; object storage plus compare-and-swap later | Datomic, XTDB, turbopuffer, Kafka, Lucene | D3, D9 |
| Hippocampal indexing; boundary episodes; gain × need consolidation | Neuroscience via [research/08](../research/08-computational-hippocampus.md) | [08](08-hippocampal-design.md) |

## Gaps no one fills

- **Shared memory across tools and machines for coding agents**, with
  provenance and permissive licensing.
- **Validity in version space:** commit ranges, dependency versions, OS and
  toolchain.
- **An independent benchmark** of multi-agent, multi-session coding memory
  with updates, contradictions, and poisoning
  ([research/05](../research/05-research-and-benchmarks.md#56-gap-and-what-to-build-a-shared-memory-coding-benchmark-sketch)).
  We build it as D1 in [05](05-evaluation-plan.md#d1-coding-agent-memory-scenarios-to-be-built-working-name-hivebench).
