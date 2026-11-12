# 01: Problem, goals, non-goals, and definitions

Status: draft v0.1 (2026-11-10). Evidence labels are defined in
[Evidence labels](#evidence-labels) at the end of this page.

## The problem

Coding agents forget almost everything between sessions. Each new session
re-reads the repository, re-runs the failing build, rediscovers the toolchain
quirk, and re-derives last week's decision. Knowledge also leaks away when work
moves between agents. That happens with parallel subagents, with a different
tool, with a teammate's agent, or with the same agent after context compaction.
Whatever is lost has to be rebuilt, at a cost in tokens, time, and sometimes
correctness.

The common workarounds each leave a gap
([research/04](../research/04-coding-agent-memory-practice.md)):

| Workaround | What works | What breaks |
|---|---|---|
| Always-loaded instruction files (`AGENTS.md`, `CLAUDE.md`, `copilot-instructions.md`) | Simple, inspectable, versioned with the code | Loaded in full every session, grows without bound, has no provenance or freshness check, and is tied to one repo and often one tool |
| Product memory features (per vendor) | Low friction, sometimes verified against code | Siloed per product and vendor, opaque, and not shared across tools or machines |
| Memory frameworks and services (Mem0, Zep/Graphiti, Letta, ...) | Richer semantics: extraction, updates, temporal edges | Usually Python services with external databases and an LLM-heavy write path ([research/03](../research/03-memory-frameworks.md)) |
| Curated knowledge vaults kept by agents and humans (markdown, an index note, a curator agent) | High quality, human-reviewable, cross-tool | The index grows (the author's own vault index is about 60 KB, roughly 15k tokens), retrieval is manual, and concurrency and staleness rely on convention |

The opportunity is a shared, durable, verifiable memory that many agents can
use, and that **pays for itself** through fewer tokens, fewer steps, and higher
task success. The research question is whether such a system is worth building,
what it should look like, and how much of it already exists.

## Objective

Build and operate a Rust-native memory system that lets multiple agents
preserve, share, update, and retrieve useful knowledge across sessions,
projects, and machines. The software for local daemon, MCP, and shared-service
paths is in tree (see [README](../README.md)); open work is measurement,
hardening, and residual risks — not “is a prototype build possible.” The
deployment range is:

- **Local:** light enough to run on a developer's machine.
- **Shared:** usable as a remote service by many agents.
- **Scalable later:** architected so it could eventually handle high
  concurrency, large collections, and distributed deployment.

The central goal is **better agent outcomes per unit of compute and tokens**,
with enough detail and provenance that agents can trust and verify what they
retrieve.

## Goals

| ID | Goal | How we will know |
|---|---|---|
| G1 | **Outcome per cost.** Raise task success, or cut tokens, steps, and time per successful task, against fair baselines | End-to-end task evaluation that counts every memory-attributable token ([05](05-evaluation-plan.md)) |
| G2 | **Verifiability.** Every memory carries provenance (who, what, when, how) and links to evidence an agent can inspect on demand | Provenance-correctness checks in the retrieval benchmark |
| G3 | **Correct over time.** Represent updates, supersession, contradictions, and validity (when something was true versus when it was recorded, and under which conditions), so stale knowledge does not pass as current | Stale-hit and contradiction metrics on scenarios with injected changes |
| G4 | **Sharing with boundaries.** Agent-private, project, team, and shared scopes, explicit promotion, and isolation between tenants | Scope-leak tests; promotion policy tests |
| G5 | **Lean local.** One binary, no required external service, no required LLM or GPU, small idle footprint, fast start | Footprint benchmarks against the provisional targets below |
| G6 | **Shared service.** The same core serves many concurrent agents remotely over MCP (Streamable HTTP) and gRPC, with authentication | Concurrency benchmarks; auth tests |
| G7 | **Durable.** Acknowledged writes survive crashes, recovery is automatic and bounded, and data exports to open formats | Crash-injection tests; recovery-time measurements |
| G8 | **Evolvable.** A path to replication and sharding stays open without committing to it now | Design review of IDs, log format, and idempotency ([04](04-architecture-and-decisions.md)) |
| G9 | **Agent-agnostic.** Works with any MCP-capable agent, without lock-in to one vendor | Integration tests with at least two agent harnesses |

### Provisional targets (hypotheses to test, not commitments)

These give the experiments something to fail against. Each will be revised
when measured ([07](07-falsification-experiments.md)).

| Metric (local mode, laptop, 100k memories) | Provisional target | Why this level |
|---|---|---|
| Recall latency p95, excluding query embedding | ≤ 50 ms | Agent turns take seconds, so this is negligible; higher values hint at a design problem |
| Write acknowledgement p95, with fsync | ≤ 20 ms | Writes happen a few times per agent turn at most |
| Idle RSS without an embedding model | ≤ 50 MB | Comparable to a small background daemon |
| Idle RSS with a small local embedding model | ≤ 300 MB | Small embedding models fit in this budget ([research/07](../research/07-embeddings-and-local-inference.md)) |
| Cold start without an embedding model | ≤ 200 ms | MCP stdio servers are often spawned per session |
| Lost acknowledged writes in crash tests | 0 | Durability is a goal, not a tuning option |
| Memory-attributable token overhead on tasks where memory does not help | ≤ 5% of task tokens | Memory must not tax tasks that don't need it |

## Non-goals for the first version

- **Not a general-purpose or vector database.** hivemind's engine is
  purpose-built for one workload: immutable appended events with derived
  indexes. It does not offer general transactions, SQL, or arbitrary schemas,
  and it does not compete on raw ANN benchmarks
  ([04](04-architecture-and-decisions.md#d2-our-own-storage-engine)).
- **No general-purpose storage engine features:** no in-place update B-trees,
  no cross-partition transactions, no query language in v0. The engine stays
  as small as the workload allows.
- **No distributed consensus, multi-region replication, or local-first CRDT
  sync in v1.** The design keeps a path open ([04](04-architecture-and-decisions.md)).
- **No parametric memory:** no fine-tuning, LoRA adapters, or persisted KV
  caches.
- **Not a code search engine.** Code is indexed by existing tools; hivemind
  stores knowledge *about* code, with citations into it.
- **Not an agent framework or orchestrator,** and not a chat-history store for
  consumer chatbots, though nothing prevents that use.
- **Memories are data, not instructions.** hivemind never rewrites an agent's
  system prompt or instruction files on its own.
- **Not a secret store.** Secret redaction on the write path is a requirement.

## Key definitions

| Term | Definition |
|---|---|
| **Principal** | An identity that reads or writes: a human, an agent instance, or a service. Every write is attributed to one |
| **Session** | One continuous run of an agent, from start to end or handoff |
| **Artifact** | An immutable, content-addressed blob such as a log, diff, transcript excerpt, file snapshot, or command output |
| **Episode** (observation) | An immutable record that something happened or was seen: "principal P, in context C, at time T, did or observed X", with artifact references. This is the **episodic** layer |
| **Claim** (memory item) | An atomic, typed statement derived from evidence. Kinds: `fact`, `decision`, `preference`, `procedure`, `solution` (problem to fix), `warning` (gotcha), `question`, `handoff`. This is the **semantic and procedural** layer |
| **Entity** | Something claims are about: repository, file, symbol, package at a version, toolchain, environment, task, person or agent, concept, error signature |
| **Edge** | A typed, time-scoped relation between entities, claims, and evidence, such as `about`, `supports`, `contradicts`, `supersedes`, `derived_from`, or `fixed_by` |
| **Scope** | A visibility namespace: agent, session, project, team, organization, or global. Every record has exactly one owning scope |
| **Applicability conditions** | Predicates on the reader's context under which a claim holds, such as repository, branch, commit range, dependency versions, OS and architecture, or toolchain |
| **Context fingerprint** | The reader's current context (repository, HEAD, lockfile digests, OS, toolchain), used to evaluate applicability |
| **Valid time** | When a claim was true in the world (SQL:2011 calls this *application time*) |
| **Record time** | When the system held the claim as current (SQL:2011 *system time*, also called transaction time) |
| **Bitemporal** | Tracking valid time and record time independently, so "what was true" and "what we believed when" are both answerable |
| **Supersede / retract / invalidate / expire / forget** | Five different end-of-life operations. Their semantics are in [03](03-memory-model.md#lifecycle-operations) |
| **Provenance** | The chain from a claim to its evidence and sources, plus the author, the method (agent-authored, LLM-extracted, human, imported, consolidated), and the time |
| **Verification status** | `unverified`, `corroborated`, `verified`, or `disputed` |
| **Trust tier** | How far a claim may be relied on and shared: `untrusted` (derived only from external content), `agent`, `corroborated`, `human` |
| **Consolidation** | Offline transformation of episodes and claims into deduplicated claims, procedures, or summaries. It never destroys the inputs |
| **Progressive retrieval** (progressive disclosure) | Returning the least useful thing first: a compact *card*, then detail, then evidence excerpts, then the raw artifact, each on request |
| **Memory-attributable tokens** | All model tokens that exist because of memory: extraction, deduplication, consolidation, query formulation, embedding (when billed), retrieved context, follow-up expansion, tool schemas, and background jobs |
| **Cost per successful task** | Total tokens (and dollars) across all attempts, divided by the number of successful tasks |
| **Acknowledged write** | A write the server has reported as committed. Durability guarantees apply to acknowledged writes only |
| **Read-your-writes** | After an agent's write is acknowledged, that agent's later reads observe it |

## The "hippocampus" framing, kept honest

The name borrows from **complementary learning systems** theory. In that
theory, the hippocampus quickly encodes specific episodes, and the neocortex
slowly consolidates regularities from them
([McClelland, McNaughton & O'Reilly, 1995](https://doi.org/10.1037/0033-295X.102.3.419)).
The **hippocampal memory indexing theory** adds that the hippocampus stores an
*index* that points to the full patterns held elsewhere
([Teyler & DiScenna, 1986](https://doi.org/10.1037/0735-7044.100.2.147)).
HippoRAG used the indexing theory to design LLM retrieval
([Gutiérrez et al., 2024](https://arxiv.org/abs/2405.14831)).

The useful translation for engineering is modest. hivemind should:

- **encode episodes fast and cheaply:** append-only evidence, no LLM required;
- **consolidate slowly and optionally:** derive claims and procedures offline;
- **retrieve through an index:** compact cards that point to full evidence.

We claim nothing about neural fidelity; the metaphor is a design mnemonic.

## Evidence labels

The repository separates evidence strength with inline labels:

| Label | Meaning |
|---|---|
| **[V]** | **Verified:** checked against a primary source (code, docs, spec, or paper) on the stated date |
| **[C]** | **Claim:** stated by a project, vendor, or authors, not independently verified |
| **[I]** | **Independent:** supported by third-party evaluation, replication, or production reports |
| **[Inf]** | **Inference:** our reasoning from the evidence |
| **[?]** | **Open:** unknown or unresolved |

For systems, we also separate **intended** capabilities, **implemented**
capabilities, and **demonstrated** capabilities
([02](02-landscape.md)).
