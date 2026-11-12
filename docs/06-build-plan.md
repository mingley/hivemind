# 06: Build plan, milestones, and open questions

> **Historical build plan** (not current product status). This page records the
> M0–M6 milestone plan written during early construction, what each milestone
> shipped, and what it deliberately did **not** prove. hivemind is no longer
> framed as a “prototype” or “v0-only” product — for today’s status, measured
> RSS, MCP path, and **residual risks**, see the
> [README](../README.md#status-and-measurements) and
> [Residual risks](../README.md#residual-risks).

Status: historical (milestones M0–M6 done 2026-11-11; RSS/MCP notes updated
2026-11-12). Written after the build decision
([04](04-architecture-and-decisions.md#decision-summary)). Each milestone ends
with something usable, a test or benchmark that would expose failure, and an
explicit list of what it does **not** prove.

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
| **M6 Engine growth** | Snapshots; memory-mapped immutable index segments; ANN for large scopes; deterministic simulation testing | The v0 limits found by F8 are lifted | — |

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
  **Partial:** format/codec versions are explicit (`SEGMENT_FORMAT` /
  `JSON_V1` = 1), covered by golden fixtures and property tests; full
  research-09 frame redesign deferred (would be a format bump).
- Evaluate a `postcard` codec (codec 2) next to readable JSON (codec 1).
- Group commit across concurrent writers through a single writer thread
  (daemon path lands in M1; explicit batching channel still optional).
- Guard free disk space before `ENOSPC`. Sweep stray temporary blob files at
  open. **Done** (`Error::DiskFull`; `blobs/tmp/` swept on open).
- Crash testing beyond `kill -9`:
  - a simulated filesystem with seeded crash schedules (evaluate
    `turmoil-fs`);
  - LazyFS on Linux CI;
  - decoder fuzzing;
  - golden fixtures for every format version.
  **Partial:** decoder/frame property tests + goldens + existing 12×
  `kill -9` ship in CI; LazyFS / turmoil-fs remain open — see
  [f13-crash-injection.md](f13-crash-injection.md).
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

**Gaps after M1 (updated 2026-11-12):**

- **RSS mitigations landed** (keydir + on-demand log load for show; interning
  of scopes / principals / entity keys; lexical token-type cap, default
  250k). **Re-measured** with
  `cargo run --release -p hivemind-engine --example rss_harness` (same seed
  as `examples/bench`; RSS via `ps -o rss=` on the harness PID):

  | Host | N claims | Terms | RSS after replay | vs ≤ 50 MB |
  |---|---|---|---|---|
  | M0 Apple M4 Pro (pre-mitigation) | 100k | ~207k | 230 MB | ❌ |
  | linux/x86_64 (post-mitigation, 2026-11-12) | 100k | 250k (cap) | **~207 MB** | ❌ |
  | linux/x86_64 (post-mitigation) | 10k | ~46k | **~30 MB** | ✅ at this size |

  **Residual:** BM25 dictionary/postings under a large synthetic vocab, plus
  keydir statement caches. `seal_lexical` does not cut RSS yet
  (`LexicalSegment::open` anon-copies and the live index stays). Further
  cheap knobs: lower `Options.max_terms`; real coding corpora share far more
  tokens than the synthetic bench. Next structural cut: search sealed
  file-backed segments and drop live postings.
- **Sandbox (Claude Code answered):** default Claude Code Bash sandbox does
  **not** cover MCP servers, so `hivemind mcp` → spawn `hivemindd` + Unix
  socket works without extra allowlists. Codex / Copilot CLI when sandboxed
  still need socket allowlists or an out-of-sandbox daemon — see
  [setup-mcp.md](setup-mcp.md#sandbox-answer-claude-code--proven-default-path).
- **Per-agent identity** is environment-provided only (trust-on-first-use
  locally); shared-service auth is separate (`hivemind serve` / OAuth).
- **Default event encoding** (JSON v1) was not revisited; postcard remains a
  follow-up.

**What this proves / does not prove:** several agents on one machine can share
a durable store through the MCP shim and change feed; Claude Code’s default
sandbox is compatible. It does not prove remote HA, that memory improves
agent outcomes (M2), or that the ≤ 50 MB RSS target holds at 100k synthetic
claims.

Working notes that guided the implementation are kept as historical in
[m1-implementation-notes.md](m1-implementation-notes.md).

## M2 status: evaluation harness (done 2026-11-11)

**Built:**

- **`crates/hivebench`:** scenario loader/validator, offline E-R metrics
  (Recall@k, stale-hit rate, abstention, applicability precision), baselines
  B0 / B1b / B1c / B2 / Oracle / Hivemind, dry-run harness with a token
  ledger stub (`docs/05` categories).
- **`scenarios/`:** twelve seed families from
  [research/04 §9](../research/04-coding-agent-memory-practice.md#9-evaluation-seed-scenarios)
  (`01`–`12`), with timeline, gold memories, distractors, and probes.
- **CLI:** `hivebench list|validate|er|dry-run`.

**What this proves / does not prove:** offline lexical retrieval quality and
baseline plumbing for F1–F4-style work. It does not run paid LLM agent loops,
Tier C coding leaderboards, or earned-memories write-path experiments yet.

Working notes: [m2-implementation-notes.md](m2-implementation-notes.md).

## M3 status: coding-agent semantics (done 2026-11-11)

**Built:**

- **Citation verification:** read-time `valid` / `drifted` / `broken` checks
  against a workspace (`content_hash` / BLAKE3); `show --workspace` / `--here`.
- **Lockfile fingerprints:** Cargo.lock, package-lock.json, Pipfile.lock, and
  poetry.lock → `dep:cargo|npm|pip/…` context keys for applicability.
- **Handoffs:** `ClaimKind::Handoff` requires `goal`, `done`, `remaining`,
  `next_step` payload fields.
- **Leases:** durable acquire / renew / release with fencing tokens; CLI
  `hivemind lease …` and daemon methods.
- **Reinforcement / expiry:** helpful feedback with evidence still reinforces;
  default `review_by` by kind when omitted (handoff 7d, question 30d,
  fact/solution/warning/procedure 180d).
- **Interop:** Copilot JSONL and markdown `import` / `export`.

**What this proves / does not prove:** staleness signals, coordination leases,
and exchange formats exist and are tested. It does not prove F5 outcome gains
against baselines, nor shared-service security (M5).

## M4 status: retrieval depth (done 2026-11-11)

**Built:**

- **Embedding spaces:** `EmbeddingSpace` + `Embedder` trait; in-process
  `HashEmbedder` (pure Rust) for tests and dry-runs; `Store::set_embedder`
  reindexes claims into an in-memory vector index.
- **Exact quantized scan:** f32 cosine and int8 codes with f32 rescoring
  (`VectorIndex`).
- **Fusion:** reciprocal-rank fusion of lexical BM25 and vector hits
  (`rank-v1`).
- **Hippocampal mechanisms (from [08](08-hippocampal-design.md)):**
  write-time key expansion (C1), entity boost (C8), temporal neighbors (C9),
  salience weighting (C3), abstention via `min_score` (C18),
  `contrasts_with` + `differs_in` (C5), supersede only when applicability
  matches (C6), ACT-R-style activation in ranking (C14).

**What this proves / does not prove:** hybrid retrieval plumbing and the cheap
deterministic hippocampal knobs. It does not ship a production embedding
model weights file (plug any `Embedder` in), ANN, or F4 online quality wins.

## M5 status: shared service (done 2026-11-11)

**Built:**

- **Tenant partitions:** `tenants/<id>/` under `HIVEMIND_ROOT`;
  `hivemind serve` opens one store per tenant (`crates/hivemind/src/tenant.rs`).
- **JSON HTTP API:** `/health`, `/v1/tenants`, `/v1/tenants/{t}/hello|rpc|changes|backup`
  (`crates/hivemind/src/http.rs`).
- **MCP Streamable HTTP:** `rmcp` `StreamableHttpService` mounted at `/mcp`
  for `--default-tenant` (in-process `HiveMcp::from_service`, not only the
  custom axum JSON RPC). Stdio MCP (`hivemind mcp`) remains for local agents.
- **OAuth 2.1:** authorization-server + protected-resource metadata
  (`/.well-known/oauth-*`); `/oauth/authorize` (auth code + PKCE S256);
  `/oauth/token` (`authorization_code`, `client_credentials`); sealed HMAC
  access tokens with a short TTL; `WWW-Authenticate` with `resource_metadata`
  on 401. Bootstrap bearer, OAuth client secret, and signing key are
  **separate** (`HIVEMIND_TOKEN` / `HIVEMIND_OAUTH_CLIENT_SECRET` /
  `HIVEMIND_SIGNING_KEY`). Shared-path write attribution uses the token
  subject (not env TOFU). **Scoped deferral (P1):** no external IdP
  federation, no dynamic client registration, no refresh tokens — short TTL
  + client_credentials cover laptop/team agents talking to this AS.
- **gRPC:** tonic `hivemind.v1.Hivemind` (`Hello`, `Rpc`, `ChangesSince`) on
  `--grpc-listen` (default port 8788); same bearer/OAuth validation.
- **Laptop sync helpers:** append-only upload queue + pull cursor JSON under
  `store/sync/` (`crates/hivemind/src/sync.rs`); change feed pull via
  `/v1/tenants/{t}/changes?from=`.
- **Segment backup:** `Store::backup_segments` copies `*.hlog` + `MANIFEST`;
  CLI `hivemind backup` and HTTP `POST …/backup`.

**What this proves / does not prove:** multi-tenant remote access with MCP
Streamable HTTP, OAuth 2.1 wire shape, gRPC, sync queue/cursor, and segment
shipping exist and are tested. It does **not** prove team-sharing outcome
gains (F11), poisoning defenses under attack (F12), or high availability.

## M6 status: engine growth (done 2026-11-11)

**Built:**

- **Snapshots:** `Store::checkpoint` writes `snapshots/<offset>.hsnap`
  (BLAKE3-sealed event prefix). Open applies the pack then
  `Log::open_from` skips re-applying those offsets while still validating the
  full log (`crates/hivemind-engine/src/snapshot.rs`).
- **Mmap immutable lexical segments:** `LexicalSegment` seals BM25
  postings to `HMLXSEG\x01` files and searches via `memmap2`
  (`indexseg.rs`); `Store::seal_lexical` writes
  `indexes/lexical-<offset>.seg`.
- **ANN for large scopes:** random-hyperplane LSH (`VectorAnn`) with exact
  cosine rescoring; `Store::build_ann` / `VectorIndex::search_ann`; recall
  prefers ANN when built.
- **Deterministic simulation:** seeded LCG crash/reopen suite
  (`tests/m6_sim.rs`) checks acknowledged offsets survive and a segment
  backup+reopen matches claim counts.

**What this proves / does not prove:** the F8 growth levers (checkpoints,
mmap segments, ANN, sim harness) exist and are tested. It does **not** prove
1M-scale replay/RSS budgets are met, nor that LSH recall quality matches a
production ANN library under all distributions.

## Open questions

| Question | Why it matters | How we'll answer it |
|---|---|---|
| Do agent sandboxes allow a Unix socket or a spawned daemon? | Decides the default local transport | **Claude Code default path: yes** (MCP outside Bash sandbox) — [setup-mcp.md](setup-mcp.md#sandbox-answer-claude-code--proven-default-path). Codex/Copilot when sandboxed: allowlist or pre-start daemon |
| How does a stdio shim get a per-agent identity? | Attribution and access control | M1 uses env (`HIVEMIND_PRINCIPAL` / `_AGENT` / `_SESSION`); authenticated credentials remain for later |
| How should writes happen without burdening agents? (agent-authored tool calls versus hooks at session end) | Write quality and token cost (F6) | M2 harness compares the two |
| Session-level taint: did the agent read untrusted content before writing? | Laundering untrusted text into trusted memory | Design in M3; attack tests in F12 |
| Which embedding model, if any, is the default? | Footprint and quality | F4 offline in M2, online in M4 |
| What erasure SLA, and is crypto-shredding enough legally? | Privacy compliance for shared use | Before M5 |
| License for the repository | Adoption and contributions | Owner decision before the first release |
| Default event encoding (JSON v1 now) | Size, speed, and canonical hashing | [research/09](../research/09-engine-design-techniques.md); not revisited in M1; still open |
