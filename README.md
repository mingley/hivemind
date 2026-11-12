# hivemind

**A collective, durable memory for AI agents: a shared computational
hippocampus, written in Rust.**

hivemind lets many agents (Claude Code, Copilot, Codex, Cursor, custom agents)
preserve, share, update, and retrieve what they learn across sessions,
projects, and machines. The goal is **better agent outcomes per unit of compute
and tokens**, with enough provenance that agents can verify what they
retrieve. It runs as a lean local daemon on a laptop and is designed to grow
into a shared service.

> **Status (2026-11-12):** research done; M0–M6 are built and tested (engine,
> daemon/MCP, `hivebench`, citations/leases/import-export, hybrid retrieval,
> shared HTTP + MCP Streamable HTTP + OAuth 2.1 + gRPC, tenant partitions,
> sync queue/cursor, segment backup, projection snapshots, mmap lexical
> segments, LSH ANN, deterministic sim). **Ops surfaces landed:** CI installs
> `protoc` then fmt/clippy/tests, Linux release artifacts, `/health` plus
> additive `/readyz` `/metrics`, JSON logging via env, and a minimal
> Dockerfile — see [docs/operations.md](docs/operations.md). Local agents
> still use `hivemind mcp` → `hivemindd`; remote/team use `hivemind serve`.
> The product is **not** fully production-ready yet (no HA, RSS vs 50 MB
> target not re-claimed, sandbox compatibility unproven). Local MCP setup:
> [docs/setup-mcp.md](docs/setup-mcp.md).

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
| 06 | [Build plan](docs/06-prototype-plan.md) | Milestones, what each proves and doesn't, open questions |
| 07 | [Falsification experiments](docs/07-falsification-experiments.md) | Prioritized experiments that could overturn design bets early |
| 08 | [Hippocampal design](docs/08-hippocampal-design.md) | Which ideas from hippocampal research become mechanisms, and how each is tested |
| — | [MCP setup](docs/setup-mcp.md) | Build `hivemind` / `hivemindd`, configure Claude Code, Codex CLI, and Copilot CLI |
| — | [Operations](docs/operations.md) | Run, backup, sync, health/metrics, auth env, release binaries, container |
| — | [hivebench scenarios](scenarios/README.md) | M2 seed scenarios for offline retrieval and harness dry-runs |

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

M0 measurements, from the engine benchmark (Apple M4 Pro, 100k synthetic
claims):

| Metric | Result |
|---|---|
| Recall within one scope | p95 1.0 ms |
| Recall across all scopes | p95 4.8 ms |
| Write acknowledged after `F_FULLFSYNC` | p95 8.9 ms |
| Replay on open | 0.8 s |
| RSS after replay | 230 MB pre-mitigation; keydir/interning/token caps landed in M1 — re-measure before claiming ≤ 50 MB |

Details, follow-ups, and milestones are in
[06](docs/06-prototype-plan.md#m0-status-engine-core-done-2026-11-10) and
[M1 status](docs/06-prototype-plan.md#m1-status-daemon-and-mcp-done-2026-11-11).

Every change runs `cargo fmt --check`, `cargo clippy --all-targets -- -D
warnings`, and `cargo test`. The test suite includes a `kill -9` crash loop
that checks every acknowledged write survives.

## Layout

```text
docs/        design documents (start with 01, then 04); setup-mcp.md; operations.md
research/    sourced research reports, one per thread
scenarios/   M2 seed scenarios for hivebench
crates/
  hivemind-core/     memory model types: claims, events, scopes, applicability, belief, hygiene
  hivemind-engine/   storage engine: log, blobs, projections, BM25, recall
  hivemind/          `hivemind` CLI, `hivemindd` daemon, and `hivemind mcp` stdio shim
  hivebench/         M2 evaluation harness (offline E-R, baselines, dry-run)
```
