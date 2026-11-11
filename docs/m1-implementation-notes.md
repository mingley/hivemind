# M1 implementation notes (historical)

Status: **implemented / historical** (2026-11-11). Milestone **M1 Daemon and
MCP** from [06](06-prototype-plan.md) is done for the daemon, MCP shim, change
feed, setup docs, and token-type caps. Keep this file as the contract snapshot
used while the work was split across crates; prefer
[06](06-prototype-plan.md#m1-status-daemon-and-mcp-done-2026-11-11) and
[setup-mcp.md](setup-mcp.md) for current status and operator docs.

Items below that were **not** finished in M1 are marked *deferred*.

## Goals for this milestone

1. `hivemindd`: single store owner, Unix socket, idle exit. **Done.**
2. `hivemind mcp`: thin stdio shim via `rmcp` (MCP 2026-07-28). **Done.**
3. Six MCP tools (deterministic order). **Done.**
4. `changes_since` change feed + read-your-writes via `min_offset`. **Done.**
5. Engine RSS mitigations: token-type caps **Done** (default 250k); scope
   names use `Arc<str>`. Compact claim rows / full interning *Deferred*.
6. Setup docs for Claude Code, Codex, and Copilot CLI. **Done**
   ([setup-mcp.md](setup-mcp.md)).
7. Update README and [06](06-prototype-plan.md) when M1 lands. **Done.**

## Process model

- One binary family:
  - `hivemind` CLI (existing commands; prefer daemon when reachable).
  - `hivemind mcp` stdio MCP server that finds or starts the daemon.
  - `hivemindd` (or `hivemind daemon`) owns the store exclusively.
- Socket: `<store>/run/hivemindd.sock` inside a `0700` directory; socket mode
  `0600`.
- Lock: existing store `LOCK` held by the daemon only.
- Idle exit: default 600s after last client disconnect; override with
  `HIVEMIND_IDLE_TIMEOUT` (seconds; `0` disables).
- Version handshake on connect; mismatch is an error.
- Group commit: daemon funnels writes through one writer path (the store's
  existing single-log lock is enough for v0; expose a channel if useful).

## Unix-socket protocol

Newline-delimited JSON request/response over a stream socket.

Request:

```json
{"id":1,"method":"remember","params":{...}}
```

Success:

```json
{"id":1,"ok":true,"result":{...}}
```

Error:

```json
{"id":1,"ok":false,"error":{"code":"not_found","message":"..."}}
```

Methods (stable names): `hello`, `ping`, `remember`, `recall`, `show`,
`retract`, `invalidate`, `link`, `feedback`, `changes_since`, `put`, `cat`,
`stats`, `shutdown` (local/testing).

`hello` returns `{protocol, version, store_id, next_offset}`.

Actor identity comes from request params / env (`HIVEMIND_PRINCIPAL`,
`HIVEMIND_AGENT`, `HIVEMIND_SESSION`), never from unauthenticated client labels
alone. Local mode remains trust-on-first-use.

## MCP tools (order is load-bearing for prompt cache)

Tool descriptions must say memory is **evidence with provenance**, never
instructions, and **current code and tests take precedence**.

1. `remember` — record a claim (optional `supersedes`, conditions, fields).
2. `recall` — search; returns cards; accepts `ctx`, `min_offset`, scopes.
3. `show` — full claim detail by id/prefix; accepts `min_offset`.
4. `retract` — mark wrong / misrecorded / duplicate.
5. `feedback` — record outcome (helpful/unhelpful/harmful/not_applicable).
6. `changes_since` — events after a cursor (`from` offset), optional filters.

## Engine API additions (`hivemind-engine`)

- `Query.min_offset: Option<u64>` — readers wait until projections reach it.
  **Done.**
- `Store::wait_for_offset(min_offset, timeout) -> Result<()>` **Done.**
- `Store::changes_since(from, limit, filter) -> Result<Vec<(u64, Event)>>`
  with optional scope / claim / kind filters. **Done.**
- Cap distinct token types in the lexical dictionary (default 250k). **Done.**
- Scope names stored as `Arc<str>` (cheap clone toward interning). **Done.**
- Compact claim projection ("keydir"): keep filter/rank fields in RAM; load
  full `ClaimData` from the log by offset on demand. *Deferred.*
- Intern principals and entity keys across projections. *Deferred.*

## Crate layout

```text
crates/hivemind-engine/   # wait/changes APIs (RSS work deferred)
crates/hivemind/          # CLI, daemon, MCP, socket client/protocol
  src/main.rs
  src/daemon.rs
  src/protocol.rs
  src/client.rs
  src/service.rs          # Store-facing handlers shared by daemon + embedded
  src/mcp.rs
docs/setup-mcp.md         # Claude Code, Codex, Copilot CLI
```

## Tests expected

- Engine: wait_for_offset, changes_since filters; existing suite still green.
  Compact-row / interning / token-cap tests await the deferred RSS work.
- Daemon: two clients share one store; idle exit; lock exclusivity; crash of
  client leaves daemon up.
- MCP: tools list order; remember→recall→show round trip over stdio (or via
  service in-process); changes_since cursor.
- CLI: talking to a running daemon (optional embedded fallback when daemon
  absent, as today).

## Out of scope for M1

Remote HTTP/gRPC, OAuth, leases/handoffs beyond claim kind, vectors, LazyFS,
postcard codec evaluation (note only), forget/purge.
