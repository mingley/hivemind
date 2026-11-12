# hivemind

**A collective, durable memory for AI agents: a shared computational
hippocampus, written in Rust.**

hivemind lets many agents (Claude Code, Copilot, Codex, Cursor, custom agents)
preserve, share, update, and retrieve what they learn across sessions,
projects, and machines. The goal is **better agent outcomes per unit of compute
and tokens**, with enough provenance that agents can verify what they
retrieve. It runs as a lean local daemon on a laptop and is designed to grow
into a shared service.

> **Status (2026-11-12):** M0–M6 are built and tested (engine, daemon/MCP,
> `hivebench`, citations/leases/import-export, hybrid retrieval, shared HTTP +
> MCP Streamable HTTP + OAuth 2.1 + gRPC, tenant partitions, sync queue/cursor,
> segment backup, projection snapshots, mmap lexical segments, LSH ANN,
> deterministic sim). **Ops surfaces landed:** CI installs `protoc` then
> fmt/clippy/tests, Linux release artifacts, `/health` plus additive `/readyz`
> `/metrics`, JSON logging via env, and a minimal Dockerfile — see
> [docs/operations.md](docs/operations.md) and [SECURITY.md](SECURITY.md).
> Local agents use `hivemind mcp` → `hivemindd`; remote/team use
> `hivemind serve`. **Local MCP path proven for Claude Code** (MCP runs
> outside its Bash sandbox): [docs/setup-mcp.md](docs/setup-mcp.md). This is
> shared-memory software suitable for real multi-agent / team use on a single
> node — **not** a claim of multi-node HA, legal certification, or closed
> outcome-eval science. See [Residual risks](#residual-risks).

## What makes it different

- **One integrated system, all Rust:** our own log-structured storage engine,
  indexes, retrieval, and protocols. It is not a glue layer over other
  services ([decisions](docs/04-architecture-and-decisions.md)).
- **Evidence, not instructions.** Every memory carries provenance, citations,
  and a trust tier. Results say whether a memory's cited code still matches,
  and the current code always wins.
- **Correct over time.** Supersession, retraction, and invalidation are
  distinct operations. Validity can depend on versions ("applies while
  `openssl-sys` is 0.9.x on macOS arm64"), not only dates.
- **Cheap by default.** No LLM is needed to write or read. Retrieval is
  code-aware BM25 (vectors optional) and returns compact cards first, with full
  evidence on demand.
- **Multi-agent by design:** scopes (agent, project, team, global), a change
  feed, leases with fencing tokens, and structured handoffs.
- **Hippocampus-inspired where it pays:** fast episodic capture, slow optional
  consolidation, and an index of cards that points to full evidence. Each
  mechanism is tested, not assumed ([08](docs/08-hippocampal-design.md)).

## Documents

| # | Document | Contents |
|---|---|---|
| 01 | [Problem and goals](docs/01-problem-and-goals.md) | Problem, goals, non-goals, provisional targets, definitions, evidence labels |
| 02 | [Landscape](docs/02-landscape.md) | What exists, what it does and doesn't do, and what we take from it |
| 03 | [Memory model](docs/03-memory-model.md) | Logical model, lifecycle operations, trust, scopes, retrieval contract, six worked coding-agent examples |
| 04 | [Architecture and decisions](docs/04-architecture-and-decisions.md) | Decision record, engine design, durability contract, staged path to distribution, risks |
| 05 | [Evaluation plan](docs/05-evaluation-plan.md) | Memory quality, system performance, and cost per successful task, measured together against fair baselines |
| 06 | [Build plan](docs/06-build-plan.md) | Historical M0–M6 milestones, what each proved / didn't, open questions |
| 07 | [Falsification experiments](docs/07-falsification-experiments.md) | Prioritized experiments that could overturn design bets early |
| 08 | [Hippocampal design](docs/08-hippocampal-design.md) | Which ideas from hippocampal research become mechanisms, and how each is tested |
| — | [MCP setup](docs/setup-mcp.md) | Build `hivemind` / `hivemindd`, configure Claude Code, Codex CLI, and Copilot CLI |
| — | [Operations](docs/operations.md) | Run, backup, sync, health/metrics, auth env, release binaries, container |
| — | [SECURITY.md](SECURITY.md) | Shared-service threat model: secrets, TLS proxy contract, tenants, tokens |
| — | [hivebench scenarios](scenarios/README.md) | M2 seed scenarios for offline retrieval and controlled token ledger |
| — | [hivebench ledger](docs/hivebench-ledger.md) | Controlled loop: measured vs estimated token categories |

The [`research/`](research/) directory holds the sourced research reports these
documents build on. Every external claim is labeled **[V]** verified, **[C]**
claim, **[I]** independent, **[Inf]** inference, or **[?]** open, with links and
the date checked (2026-11-10).

## Try it

```bash
cargo build --release
alias hm=./target/release/hivemind   # the store defaults to ~/.hivemind/store; override with --dir
# ./target/release/hivemindd owns the store; `hm mcp` finds or starts it for agents

hm remember "Linking fails with _SSL_CTX_new not found on macOS arm64; set OPENSSL_DIR to brew openssl@3" \
  --kind solution --scope project:acme/api \
  --about pkg:cargo/openssl-sys --when os=macos --when 'dep:cargo/openssl-sys=>=0.9.100, <0.10' \
  --field fix='export OPENSSL_DIR=$(brew --prefix openssl@3)' \
  --quote 'ld: symbol(s) not found for architecture arm64'

hm recall _SSL_CTX_new not found --here --ctx dep:cargo/openssl-sys=0.9.103
hm show <id-prefix>          # full claim: provenance, belief, links
hm recall _SSL_CTX_new --ctx os=linux   # → no matching memories (the fix doesn't apply)
```

Other commands: `retract`, `invalidate`, `link`, `feedback`, `put`, `cat`,
`log` (the change feed), `stats`, `verify`, `mcp`, and `daemon`. Run
`hm --help` for the rest. Agent wiring:
[docs/setup-mcp.md](docs/setup-mcp.md).

## Status and measurements

M0 latency (Apple M4 Pro, 100k synthetic claims) still holds as the
published latency baseline. **RSS was re-measured** after keydir / interning /
token-type caps (2026-11-12, linux/x86_64 Xeon cloud agent, release, no
embedder) with
`cargo run --release -p hivemind-engine --example rss_harness -- <N>`
(same generator seed as `examples/bench`; RSS = `ps -o rss=` for that PID):

| Metric | Result |
|---|---|
| Recall within one scope (M0, M4 Pro) | p95 1.0 ms |
| Recall across all scopes (M0, M4 Pro) | p95 4.8 ms |
| Write acknowledged after `F_FULLFSYNC` (M0, M4 Pro) | p95 8.9 ms |
| Replay on open (M0 / post-mitigation linux) | 0.8 s / ~1.9 s |
| RSS after replay, 100k claims | **~207 MB** post-mitigation (M0 pre-mitigation 230 MB on M4 Pro) — ≤ 50 MB ❌ |
| RSS after replay, 10k claims | **~30 MB** (under the 50 MB line at this size) |

**Residual:** in-memory BM25 dictionary (hits the 250k term cap on this
synthetic vocab) plus keydir rows (statement caches, applicability maps).
`Store::seal_lexical` writes a segment file but today’s
`LexicalSegment::open` copies into an anonymous map and does not free the
live heap, so sealing is **not** an RSS win yet. Cheap operator knob:
lower `Options.max_terms` for adversarial vocab; next real cut needs
file-backed mmap search that drops live postings (or a denser keydir).

Details: [06](docs/06-build-plan.md#m0-status-engine-core-done-2026-11-10)
and [M1 status](docs/06-build-plan.md#m1-status-daemon-and-mcp-done-2026-11-11).

Every change runs `cargo fmt --check`, `cargo clippy --all-targets -- -D
warnings`, and `cargo test`. The test suite includes a `kill -9` crash loop
that checks every acknowledged write survives.

## Residual risks

Honest gaps — none of these are presented as “fully production certified”:

| Risk | Status |
|---|---|
| **Multi-node HA** | Explicit NON-goal. One node + restore from shipped segment backups ([operations.md](docs/operations.md)). No leader election / multi-region active-active. |
| **IdP / DCR** | Shared OAuth 2.1 AS is in-process (PKCE + client_credentials + refresh_token + RFC 7591 DCR, short access TTL, separated secrets). DCR clients persist under `{HIVEMIND_ROOT}/oauth/dcr_clients.json`; refresh handles under `{HIVEMIND_ROOT}/oauth/refresh_tokens.json`. External IdP federation remains **P1**. |
| **Footprint at 100k** | Post-mitigation RSS ~**207 MB** vs ≤ **50 MB** target (10k ~30 MB). 1M-scale RSS/replay budgets unproven. |
| **Embeddings default** | **Lexical-only** remains the lean default (F4): no embedder required for remember/recall. Optional Model2Vec via `--features embeddings` + `--embedder model2vec` ([operations.md](docs/operations.md#embeddings-optional-lean-default-is-lexical-only)); hybrid ≥10% win not evidenced in-repo. |
| **F1–F12 research** | Mechanisms for trust, citations, hybrid recall, etc. ship; outcome / attack-efficacy measurements (F1–F12) remain open research, not ship blockers. |
| **LazyFS CI** | Frame goldens, property tests, and 12× `kill -9` run in CI; **LazyFS** power-loss injection is **not** wired into CI ([f13-crash-injection.md](docs/f13-crash-injection.md)). |
| **LICENSE** | Repository `LICENSE` file is **pending** owner choice (blocked before a tagged “production” release claim). |

Threat model and secure defaults: [SECURITY.md](SECURITY.md). Operator runbook:
[docs/operations.md](docs/operations.md).

## Layout

```text
docs/        design documents (start with 01, then 04); setup-mcp.md; operations.md
research/    sourced research reports, one per thread
scenarios/   M2 seed scenarios for hivebench
crates/
  hivemind-core/     memory model types: claims, events, scopes, applicability, belief, hygiene
  hivemind-engine/   storage engine: log, blobs, projections, BM25, recall
  hivemind/          `hivemind` CLI, `hivemindd` daemon, and `hivemind mcp` stdio shim
  hivebench/         M2 evaluation harness (offline E-R, baselines, token ledger)
```
