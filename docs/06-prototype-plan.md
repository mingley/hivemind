# 06: Build plan, milestones, and open questions

Status: v0.4 (2026-11-11). M0 and M1 are done; M2 is next. Written after the
build decision ([04](04-architecture-and-decisions.md#decision-summary)). Each
milestone ends with something usable, a test or benchmark that would expose
failure, and an explicit list of what it does **not** prove.

## Smallest useful implementation

**M0 + M1:** a local memory daemon that agents can use over MCP. It provides:

- a durable log (our own engine);
- typed claims with provenance and supersession;
- code-aware BM25 recall that returns compact cards;
- scopes;
- a change feed.

That is enough for two coding agents on one machine to share debugging
knowledge and handoffs, which is the cheapest real test of the idea.

## Milestones

| Milestone | Deliverable | Proves | Does not prove |
|---|---|---|---|
| **M0 Engine core** | Cargo workspace. `hivemind-core` (types, events, belief policy). `hivemind-engine` (segmented log with CRC32C and group-commit fsync, torn-tail recovery, blob store, in-memory projections, code-aware BM25, recall with filters and cards). `hivemind` CLI in embedded mode. Unit tests, recovery fault injection, a `kill -9` loop test | The log and projection design is correct and durable on a laptop; recall works on real claims | That memory helps agents; performance at 1M |
| **M1 Daemon and MCP** | `hivemindd` (single owner, Unix socket, idle exit). `hivemind mcp` stdio shim (`rmcp`, current MCP spec). Five or six tools. `changes_since`. Read-your-writes offsets. Setup docs for Claude Code, Codex, and Copilot CLI | Several agents on one machine can share memory safely; shim overhead (F9) | Remote or team use |
| **M2 Evaluation harness** | Scenario format and 12 seed scenarios ([research/04](../research/04-coding-agent-memory-practice.md#9-evaluation-seed-scenarios)). A controlled agent harness with a token ledger. Baselines B0, B1b, B1c, B2, and the oracle. Offline retrieval benchmark | Headroom and where hivemind adds value (F1–F3); lexical quality (F4, offline) | Long-horizon effects; multi-team use |
| **M3 Coding-agent semantics** | Citations with read-time verification (`valid`, `drifted`, `broken`). Applicability from a context fingerprint, including lockfile parsing for Cargo, npm, and pip. Leases with fencing; handoffs. Reinforcement on validated use and expiry. Copilot JSONL and markdown import and export | Staleness and conflict handling beat baselines (F5); progressive disclosure (F7) | Shared-service security |
| **M4 Retrieval depth** | Embedding spaces (pure-Rust inference), exact quantized vector scan, fusion. Hippocampus-inspired mechanisms chosen in [08](08-hippocampal-design.md) | Hybrid search earns its footprint (F4 online); graph-style recall for multi-hop (F10) | Scale beyond one node |
| **M5 Shared service** | MCP Streamable HTTP and gRPC with OAuth 2.1. One partition per tenant. Laptop upload queue and cursor pull. Backup by shipping segments | Team sharing value and harm (F11); poisoning defenses (F12) | High availability |
| **M6 Engine growth** (triggered) | Snapshots; memory-mapped immutable index segments; ANN for large scopes; deterministic simulation testing | The v0 limits found by F8 are lifted | — |

F13 (crash safety) runs from M0 onward and blocks every release.

## M0 status: engine core (done 2026-11-10)

**Built** (`crates/`):

- **`hivemind-core`:** IDs (UUIDv7, BLAKE3), scopes, claims, events, the
  applicability evaluator (exact keys, alternatives, prefixes, semver ranges
  for `dep:` keys), belief labels, and write hygiene (secret detection,
  invisible-character stripping).
- **`hivemind-engine`:**
  - the segmented log, with one checksummed record per commit, flushed
    before acknowledgement;
  - recovery that truncates a torn tail and refuses to open on corruption;
  - the BLAKE3 blob store;
  - projections and the `belief-v1` policy (independent evidence groups,
    disputes, taint);
  - code-aware BM25;
  - recall with filters, applicability, two-phase ranking, and token-budgeted
    cards;
  - similar-claim detection on write, idempotency keys, the change feed,
    stats, and verify.
- **`hivemind` CLI:**
  - `init`, `remember`, `recall`, `show`;
  - `retract`, `invalidate`, `link`, `feedback`;
  - `put`, `cat`, `log`, `stats`, `verify`.

**Hardened after an independent code review** (all 7 findings fixed, each
with a regression test):

- **Replay safety:** decode-before-append, plus validation of scopes, payload
  depth, and timestamp ranges.
- **Belief policy `belief-v1`:**
  - writer labels never grant human trust;
  - authorship doesn't lift taint;
  - derived claims are capped by their inputs;
  - `supports` edges are credited to whoever adds them.
- **Applicability:** prerelease toolchains match by their base version.
- **Recovery:** truncations are recorded in the log.
- **Durability:** blob and directory fsyncs; the MANIFEST is written last.

**Tested:** 70 tests.

- Torn-tail truncation at every byte, and bit flips at every byte of the last
  record.
- Corruption followed by valid data is refused.
- Every worked example in [03](03-memory-model.md#worked-examples).
- Replay determinism.
- A `kill -9` loop: 12 kills mid-write, every acknowledged write intact.

**First measurements** (Apple M4 Pro, macOS 26.7, release build, 100k
synthetic claims across 20 scopes, synthetic vocabulary of 207k terms;
`cargo run --release -p hivemind-engine --example bench`):

| Metric | Result | Provisional target ([01](01-problem-and-goals.md#provisional-targets-hypotheses-to-test-not-commitments)) |
|---|---|---|
| Recall within one scope, p50 / p95 / p99 | 0.64 / 1.04 / 1.22 ms | p95 ≤ 50 ms ✅ |
| Recall across all scopes, p50 / p95 | 4.6 / 4.8 ms | — |
| Write acknowledged after `F_FULLFSYNC`, p50 / p95 / p99 | 5.5 / 8.9 / 10.4 ms | p95 ≤ 20 ms ✅ |
| Replay of 100k claims on open | 0.80 s | Snapshots once replay exceeds 2 s |
| Bulk ingest without fsync (each write also finds similar claims) | about 2,000 claims/s | — |
| RSS after replay | 230 MB | ≤ 50 MB ❌ |

**The RSS miss is an F8 finding.** The v0 projections hold every claim's full
data in memory, and the synthetic vocabulary is large. The planned fix is
three changes (still open after M1; see
[M1 status](#m1-status-daemon-and-mcp-done-2026-11-11)):

- **Compact rows** ("keydir"): filter and rank fields stay in memory, and
  full claims are read from the log by offset on demand.
- **Interning** of scopes, principals, and entity keys.
- **Token-type caps.**

Research 09 independently predicted this, and set a budget of about 200k items
held in memory per store until on-disk index segments land
([research/09](../research/09-engine-design-techniques.md#1-summary-decisions-this-research-implies)).

**Follow-ups before the first tagged release** (the log format is free to
change until then):

- Harden the frame format, per
  [research/09 §4](../research/09-engine-design-techniques.md#4-wal-and-blob-store-correctness-checklist-and-test-strategy):
  a frame-type byte, a masked CRC, a segment epoch, and a header version.
- Evaluate a `postcard` codec (codec 2) next to readable JSON (codec 1).
- Group commit across concurrent writers through a single writer thread
  (daemon path lands in M1; explicit batching channel still optional).
- Guard free disk space before `ENOSPC`. Sweep stray temporary blob files at
  open.
- Crash testing beyond `kill -9`:
  - a simulated filesystem with seeded crash schedules (evaluate
    `turmoil-fs`);
  - LazyFS on Linux CI;
  - decoder fuzzing;
  - golden fixtures for every format version.
- `forget`: erasure with a physical purge of log payloads, blobs, and
  projections (M3).

## M1 status: daemon and MCP (done 2026-11-11)

**Built** (`crates/hivemind`, engine APIs):

- **`hivemindd`:** single store owner; Unix socket at
  `<store>/run/hivemindd.sock` (`0700` run dir, `0600` socket); version
  handshake (`hivemind-socket-v1`); idle exit via `HIVEMIND_IDLE_TIMEOUT`
  (default 600s, `0` disables).
- **`hivemind mcp`:** stdio MCP shim via `rmcp`; finds or starts `hivemindd`.
- **Six MCP tools** (fixed order): `remember`, `recall`, `show`, `retract`,
  `feedback`, `changes_since`. Server `instructions` and tool text state the
  evidence-not-instructions rule (current code wins).
- **Change feed / read-your-writes:** `Store::changes_since` with optional
  scope / claim / kind filters; `Query.min_offset` and
  `Store::wait_for_offset` so a reader can wait for a prior write offset.
- **Actor env:** `HIVEMIND_DIR`, `HIVEMIND_PRINCIPAL`, `HIVEMIND_AGENT`,
  `HIVEMIND_SESSION` (plus idle timeout on the daemon).
- **Setup docs:** [setup-mcp.md](setup-mcp.md) for Claude Code, Codex CLI, and
  Copilot CLI.

**Still open from the M1 brief (honest gaps):**

- **RSS mitigations** called out after M0 (compact claim rows / keydir,
  interning of scopes and principals, token-type caps) are **not** landed.
  The 230 MB / 100k-claim M0 measurement still stands; no new RSS number is
  claimed here.
- **Sandbox questions** remain unanswered: whether Claude Code, Codex, and
  Copilot CLI sandboxes allow a Unix socket or a spawned daemon. The setup
  doc records the workaround of starting `hivemindd` outside the sandbox.
- **Per-agent identity** is environment-provided only (trust-on-first-use
  locally); no authenticated credentials yet.
- **Default event encoding** (JSON v1) was not revisited; postcard remains a
  follow-up.
- Remote HTTP/gRPC, OAuth, leases/handoffs beyond claim kind, vectors, and
  `forget` stay out of scope (as planned).

**What this proves / does not prove:** several agents on one machine can share
a durable store through the MCP shim and change feed. It does not prove remote
or team use, that memory improves agent outcomes (M2), or that the laptop RSS
target is met.

Working notes that guided the implementation are kept as historical in
[m1-implementation-notes.md](m1-implementation-notes.md).

## Open questions

| Question | Why it matters | How we'll answer it |
|---|---|---|
| Do agent sandboxes (cloud agents, restricted CLIs) allow a Unix socket or a spawned daemon? | Decides the default local transport | Still open after M1; try each sandbox with [setup-mcp.md](setup-mcp.md) |
| How does a stdio shim get a per-agent identity? | Attribution and access control | M1 uses env (`HIVEMIND_PRINCIPAL` / `_AGENT` / `_SESSION`); authenticated credentials remain for later |
| How should writes happen without burdening agents? (agent-authored tool calls versus hooks at session end) | Write quality and token cost (F6) | M2 harness compares the two |
| Session-level taint: did the agent read untrusted content before writing? | Laundering untrusted text into trusted memory | Design in M3; attack tests in F12 |
| Which embedding model, if any, is the default? | Footprint and quality | F4 offline in M2, online in M4 |
| What erasure SLA, and is crypto-shredding enough legally? | Privacy compliance for shared use | Before M5 |
| License for the repository | Adoption and contributions | Owner decision before the first release |
| Default event encoding (JSON v1 now) | Size, speed, and canonical hashing | [research/09](../research/09-engine-design-techniques.md); not revisited in M1; still open |
