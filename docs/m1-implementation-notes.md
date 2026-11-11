# M1 implementation notes (working)

Status: in progress. Implements milestone **M1 Daemon and MCP** from
[06](06-prototype-plan.md). Agents should treat this file as the shared
contract while work is split across crates.

## Goals for this milestone

1. `hivemindd`: single store owner, Unix socket, idle exit.
2. `hivemind mcp`: thin stdio shim via `rmcp` (MCP 2026-07-28).
3. Six MCP tools (deterministic order).
4. `changes_since` change feed + read-your-writes via `min_offset`.
5. Engine RSS mitigations: compact claim rows, string interning, token-type caps.
6. Setup docs for Claude Code, Codex, and Copilot CLI.
7. Update README and [06](06-prototype-plan.md) when M1 lands.

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
- `Store::wait_for_offset(min_offset, timeout) -> Result<()>`
- `Store::changes_since(from, limit, filter) -> Result<Vec<(u64, Event)>>`
  with optional scope / claim / kind filters.
- Compact claim projection ("keydir"): keep filter/rank fields in RAM; load
  full `ClaimData` from the log by offset on demand (`show`, card expansion).
- Intern scopes, principals, and entity keys.
- Cap distinct token types in the lexical dictionary (drop rare tail tokens
  beyond a configurable cap; default high enough not to hurt small stores).

## Crate layout

```text
crates/hivemind-engine/   # RSS + wait/changes APIs
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

- Engine: wait_for_offset, changes_since filters, compact-row round-trip,
  interning/token-cap smoke tests, existing suite still green.
- Daemon: two clients share one store; idle exit; lock exclusivity; crash of
  client leaves daemon up.
- MCP: tools list order; remember→recall→show round trip over stdio (or via
  service in-process); changes_since cursor.
- CLI: talking to a running daemon (optional embedded fallback when daemon
  absent, as today).

## Out of scope for M1

Remote HTTP/gRPC, OAuth, leases/handoffs beyond claim kind, vectors, LazyFS,
postcard codec evaluation (note only), forget/purge.
