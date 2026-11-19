# hivemind

**A collective, durable memory for AI agents: a shared computational
hippocampus, written in Rust.**

hivemind lets many agents (Claude Code, Copilot, Codex, Cursor, custom agents)
preserve, share, update, and retrieve what they learn across sessions,
projects, and machines. The goal is **better agent outcomes per unit of compute
and tokens**, with enough provenance that agents can verify what they
retrieve. It runs as a lean local daemon on a laptop and is designed to grow
into a shared service.

> **Status (2026-11-16):** M0–M6 are built and tested (engine, daemon/MCP,
> `hivebench`, citations/leases/import-export, hybrid retrieval, shared HTTP +
> MCP Streamable HTTP + OAuth 2.1 + gRPC, tenant partitions, sync queue/cursor,
> segment backup, projection snapshots, mmap lexical segments, LSH ANN,
> deterministic sim). **Ops surfaces landed:** CI installs `protoc` then
> fmt/clippy/tests, Linux release artifacts, `/health` plus additive `/readyz`
> `/metrics`, JSON logging via env, and a minimal Dockerfile — see
> [docs/operations.md](docs/operations.md) and [SECURITY.md](SECURITY.md).
> Local agents use `hivemind mcp` → `hivemindd`; remote/team use
> `hivemind serve`. **Local MCP path proven for Claude Code** (MCP runs
> outside its Bash sandbox): [docs/setup-mcp.md](docs/setup-mcp.md). Tenant grants, admin-only OAuth approval/backup, complete restore, and erasure failure handling are enforced. This is
> shared-memory software for bounded single-node team deployments — **not** a claim of multi-node HA, legal certification, or closed
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

Other commands: `brief` (Turn-0 hook briefing), `retract`, `invalidate`,
`link`, `feedback`, `put`, `cat`, `log` (the change feed), `stats`, `verify`,
`mcp`, and `daemon`. Run `hm --help` for the rest. Agent wiring:
[docs/setup-mcp.md](docs/setup-mcp.md).

## Status and measurements

Native downloads: [v0.2.0](https://github.com/mingley/hivemind/releases/tag/v0.2.0)
for Apple Silicon / Intel Macs and Linux x86-64 / ARM64. See the
[quickstart](docs/quickstart.md) to install, run a one-minute test, and connect
Codex or Claude Code without compiling Rust.

Release-build measurements on Linux/x86_64, 100k synthetic claims across
20 scopes, lexical-only:

| Operation | p95 |
|---|---|
| Recall within one scope | 1.10 ms |
| Recall across scopes | 8.29 ms |
| Expand full claim by id | 0.154 ms |
| Write acknowledged after fsync | 3.07 ms |

Replay-only RSS is **117.0 MiB at 100k** and **882.2 MiB at 1M**; replay takes
2.26 s / 23.99 s. At 20k, same-host scoped recall improved ~1.9× and global
recall ~2.9×; 100k replay memory fell ~44%. The original 50 MB laptop goal
remains unmet at 100k. The supported deployment envelope is explicit:
default 100k admission cap, at least 512 MiB provisioned per active store plus
request/backup overhead. [Full methodology and operating limits](docs/performance.md).

CI gates formatting, Clippy, workspace tests, 100k latency/RSS budgets,
1,000-cycle SIGKILL survival, and LazyFS cache-loss tests. Every native release
archive is extracted and smoke-tested for remember/recall, verify,
backup/restore, and daemon sharing before publication.

## Residual risks

Honest gaps — none of these are presented as “fully production certified”:

| Risk | Status |
|---|---|
| **Multi-node HA** | Explicit NON-goal. One node + restore from complete log/manifest/blob backups ([operations.md](docs/operations.md)). No leader election / multi-region active-active. |
| **IdP / DCR** | Shared OAuth 2.1 AS is in-process (PKCE + client_credentials + refresh_token + RFC 7591 DCR, short access TTL, separated secrets). DCR clients persist under `{HIVEMIND_ROOT}/oauth/dcr_clients.json`; refresh handles under `{HIVEMIND_ROOT}/oauth/refresh_tokens.json`. DCR is disabled by default; enabling it still requires bootstrap admin registration/approval. Server-owned tenant grants deny cross-tenant access. External IdP federation is an explicit **NON-goal** (in-process AS only). |
| **Capacity** | Default admission cap: 100k claims/store; measured replay RSS 117.0 MiB (256 MiB CI benchmark budget). Opt-in 1M: 882.2 MiB / 23.99 s replay. Original 50 MB laptop target remains unmet; see [capacity envelope](docs/operations.md#supported-capacity-and-measured-envelope). |
| **Embeddings default** | **Lexical-only** remains the lean default (F4): no embedder required for remember/recall. Optional Model2Vec via `--features embeddings` + `--embedder model2vec` ([operations.md](docs/operations.md#embeddings-optional-lean-default-is-lexical-only)); hybrid ≥10% win not evidenced in-repo. |
| **F1–F12 research** | Mechanisms for trust, citations, hybrid recall, etc. ship; outcome / attack-efficacy measurements (F1–F12) remain open research, not ship blockers. |
| **Durability CI** | Frame goldens, property tests, 12-cycle `kill -9` smoke plus a 1,000-cycle Linux soak, and a pinned LazyFS drop-unsynced-data job gate CI/release packaging. The LazyFS test includes an unsynced negative control, acknowledged claims/blobs, erasure, and reopen verification ([F13](docs/f13-crash-injection.md)). |

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

## License

MIT — see [LICENSE](LICENSE).
