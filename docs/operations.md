# Operations

Operator guide for running hivemind as a local daemon or shared HTTP/gRPC
service. **Ops surfaces** (health probes, metrics, release packaging, container
image) are in place. For product-level residual risks (HA, IdP/DCR, RSS,
F1–F12, LazyFS CI, LICENSE), see the
[README residual risks](../README.md#residual-risks) — this runbook does not
claim multi-node HA or legal certification.

## Modes

| Mode | Command | Persist | Auth |
|---|---|---|---|
| Local daemon | `hivemindd` or `hivemind daemon` | `HIVEMIND_DIR` (default `~/.hivemind/store`) | Unix socket modes only (`0700`/`0600`) |
| Local MCP shim | `hivemind mcp` | same store via daemon | same; see [setup-mcp.md](setup-mcp.md) |
| Shared service | `hivemind serve` | `HIVEMIND_ROOT/tenants/<id>/` | Bearer bootstrap token or OAuth 2.1 |

## Environment variables

### Logging / observability

| Variable | Purpose | Default |
|---|---|---|
| `RUST_LOG` | `tracing` filter (`info`, `hivemind=debug`, …) | `info` |
| `HIVEMIND_LOG_FORMAT` | `json` / `true` / `1` for JSON logs on stderr; otherwise plain text | unset (plain) |

Daemon, `hivemind serve`, and gRPC all call the same tracing init.

### Local daemon / CLI / MCP

| Variable | Purpose | Default |
|---|---|---|
| `HIVEMIND_DIR` | Store directory | `~/.hivemind/store` |
| `HIVEMIND_IDLE_TIMEOUT` | Daemon idle exit seconds; `0` disables | `600` |
| `HIVEMIND_PRINCIPAL` | Author principal on writes | `$USER` / `anonymous` |
| `HIVEMIND_AGENT` | Agent name on writes | unset |
| `HIVEMIND_SESSION` | Session id on writes | unset |
| `HIVEMIND_WORKSPACE` | Workspace root for citation checks (CLI) | cwd |

### Shared service (`hivemind serve`)

| Variable | Purpose | Default |
|---|---|---|
| `HIVEMIND_ROOT` | Multi-tenant root (`tenants/<id>/`) | `~/.hivemind` |
| `HIVEMIND_TOKEN` | Bootstrap / admin bearer (**required**; must differ from the other two secrets) | — |
| `HIVEMIND_OAUTH_CLIENT_SECRET` | OAuth confidential client secret (**required**; distinct) | — |
| `HIVEMIND_SIGNING_KEY` | Access-token HMAC signing key (**required**; distinct) | — |
| `HIVEMIND_DEFAULT_TENANT` | Tenant behind `/mcp` | `default` |
| `HIVEMIND_PUBLIC_BASE` | Public base URL in OAuth metadata | `http://{listen}` |
| `HIVEMIND_OAUTH_CLIENT_ID` | Pre-registered confidential client id | `hivemind-cli` |
| `HIVEMIND_ACCESS_TOKEN_TTL_SECS` | Access-token lifetime (short TTL) | `300` |
| `HIVEMIND_REFRESH_TOKEN_TTL_SECS` | Refresh-token lifetime (`authorization_code` / `refresh_token`) | `604800` (7d) |
| `HIVEMIND_REQUIRE_TLS` | `1`/`true`/`yes` — refuse start unless `PUBLIC_BASE` is `https://` | unset |
| `HIVEMIND_GRPC_LISTEN` | gRPC listen address | same host as HTTP, port `8788` |

Flags: `--listen` (default `127.0.0.1:8787`), `--no-grpc`, `--token`,
`--oauth-client-secret`, `--signing-key`, `--access-token-ttl-secs`,
`--refresh-token-ttl-secs`, `--require-tls`, `--root`.

**OAuth tokens:** `client_credentials` returns a short-lived access token only
(machine agents re-auth with the client secret). Interactive
`authorization_code` + PKCE also returns a `refresh_token`; confidential
clients exchange it via `grant_type=refresh_token` (refresh rotates). See
[SECURITY.md](../SECURITY.md).

**TLS:** `hivemind serve` speaks plaintext HTTP/gRPC. Production MUST terminate
TLS at a reverse proxy (or equivalent), set
`HIVEMIND_PUBLIC_BASE=https://…`, and set `HIVEMIND_REQUIRE_TLS=1`. See
[SECURITY.md](../SECURITY.md).

## Health and metrics (HTTP)

Open (no auth). `/health` is the stable liveness probe; `/readyz` and
`/metrics` are additive. `/healthz` is an optional alias of `/health` only
(not a rename).

| Path | Role |
|---|---|
| `GET /health` | Liveness — process is up |
| `GET /readyz` | Readiness — default tenant store is open; returns `next_offset` |
| `GET /metrics` | Prometheus text (`hivemind_*` counters / histograms / gauges) |
| `GET /healthz` | Optional alias of `/health` |

Example metrics:

- `hivemind_http_requests_total{method,path,status}`
- `hivemind_http_request_duration_seconds{method,path}` (histogram)
- `hivemind_remember_total` / `hivemind_recall_total`
- `hivemind_log_offset{tenant}` — next append offset for open stores

```bash
curl -sS "http://127.0.0.1:8787/health"
curl -sS "http://127.0.0.1:8787/readyz"
curl -sS "http://127.0.0.1:8787/metrics"
```

## Run: shared service

```bash
export HIVEMIND_TOKEN="$(openssl rand -hex 32)"
export HIVEMIND_OAUTH_CLIENT_SECRET="$(openssl rand -hex 32)"
export HIVEMIND_SIGNING_KEY="$(openssl rand -hex 32)"
export HIVEMIND_ROOT="$HOME/.hivemind"
cargo build --release -p hivemind
./target/release/hivemind serve --listen 127.0.0.1:8787 --root "$HIVEMIND_ROOT"
# production-ish:
# export HIVEMIND_PUBLIC_BASE=https://memory.example
# export HIVEMIND_REQUIRE_TLS=1
# optional JSON logs:
# HIVEMIND_LOG_FORMAT=json RUST_LOG=info ./target/release/hivemind serve ...
```

Authenticate JSON API / MCP with `Authorization: Bearer $HIVEMIND_TOKEN`, or
obtain an OAuth access token via `/oauth/token` using
`HIVEMIND_OAUTH_CLIENT_SECRET` (client credentials; auth code + PKCE with
refresh; or `refresh_token` grant). Access tokens are short-TTL; refresh
tokens last until `HIVEMIND_REFRESH_TOKEN_TTL_SECS` and rotate on use.
Shared-path write attribution uses the token subject, not env TOFU. See
[SECURITY.md](../SECURITY.md) and [setup-mcp.md](setup-mcp.md).

## Backup and sync

**Segment backup** (durable log segments only):

```bash
# single-store layout
hivemind backup /path/to/backup-dir --dir "$HIVEMIND_DIR"

# multi-tenant
hivemind backup /path/to/backup-dir --tenant acme --root "$HIVEMIND_ROOT"
# or HTTP (auth required):
curl -X POST -H "Authorization: Bearer $HIVEMIND_TOKEN" \
  -H 'content-type: application/json' \
  -d '{"dest":"/path/to/backup-dir"}' \
  "http://127.0.0.1:8787/v1/tenants/acme/backup"
```

**Change feed / sync cursor:** `GET /v1/tenants/{tenant}/changes?from=<offset>`
(or daemon/MCP `changes_since`). Clients advance `from` using the highest
offset received. `readyz` / `hivemind_log_offset` expose the server's next
append offset for lag checks.

Restore: stop writers, copy segments into an empty store directory (or restore
the whole tenant dir), then open with `hivemind` / `serve`. Prefer offline
restore; live restore is not supported.

HA is restore-from-segment-backup + this runbook (no multi-node failover).
Durability release-gate coverage (kill-9, frame/codec properties, goldens) and
the LazyFS power-loss gap: [f13-crash-injection.md](f13-crash-injection.md).

## Release binaries (CI)

- **PR / every push to `main`:** `.github/workflows/ci.yml` — installs
  `protoc`, then `cargo fmt --check`,
  `cargo clippy --workspace --all-targets --locked -- -D warnings`,
  `cargo test --workspace --locked` on Ubuntu and macOS.
- **Release packaging:** `.github/workflows/release.yml` — installs `protoc`,
  then on `main` pushes, `v*` tags, and `workflow_dispatch` builds Linux
  release binaries and uploads artifact `hivemind-linux-x86_64` (`hivemind`,
  `hivemindd`, `SHA256SUMS`, and a `.tar.gz`).

Local equivalent:

```bash
cargo build --release --locked -p hivemind
# artifacts: target/release/hivemind  target/release/hivemindd
```

## Container

```bash
docker build -t hivemind:local .
docker run --rm -p 8787:8787 -p 8788:8788 \
  -e HIVEMIND_TOKEN="$(openssl rand -hex 32)" \
  -e HIVEMIND_OAUTH_CLIENT_SECRET="$(openssl rand -hex 32)" \
  -e HIVEMIND_SIGNING_KEY="$(openssl rand -hex 32)" \
  -e HIVEMIND_LOG_FORMAT=json \
  -v hivemind-data:/var/lib/hivemind \
  hivemind:local
```

The image entrypoint is `hivemind serve --listen 0.0.0.0:8787 --root /var/lib/hivemind`.
Pass distinct bootstrap / client-secret / signing-key env vars. Override with
e.g. `docker run … hivemind:local daemon` for a single-store daemon (publish no
HTTP ports in that case; use a volume for `HIVEMIND_DIR`).

## Erasure / forget (privacy SLA)

`forget` is the only operation that **rewrites history**. It is for privacy,
compliance, or secrets — not for correcting wrong knowledge (use `retract` /
`invalidate` / supersede instead). See [03-memory-model.md](03-memory-model.md)
lifecycle table.

### What forget does

1. Appends an audited `claim_forgotten` tombstone (`claim`, `reason`, actor,
   time, and any exclusively referenced blob hashes).
2. Redacts the original `claim_asserted` log payload in place to a
   `[forgotten]` stub (statement, evidence, payload, and scope cleared).
3. Removes the claim from lexical / vector projections and entity indexes.
   Forgotten claims are **never** returned by `recall`, including with
   `--history` / `history: true`.
4. Deletes blob files that nothing else still references (content-addressed
   store; physical delete). Crypto-shredding is not used today because blobs
   are not per-key encrypted; delete is the erasure mechanism that matches
   the current blob design.
5. Invalidates projection snapshots so a reopen cannot rebuild from a pack
   that still held the pre-redaction bytes.

### Operator steps

```bash
# Local store / daemon
hivemind forget clm_<id_or_prefix> --reason "GDPR erasure request #123"

# Via MCP (stdio or Streamable HTTP /mcp)
# tool: forget  args: { "id": "clm_…", "reason": "…" }

# Daemon JSON-RPC method name: forget
```

Require a non-empty `--reason` (stored on the tombstone for audit). Re-running
`forget` on an already-forgotten claim is idempotent and re-attempts physical
purge if a prior run was interrupted.

### SLA

| Guarantee | Bound |
|---|---|
| Online store (this process holds the lock) | Purge of log payload, projections, and exclusive blobs completes in the `forget` call before the receipt is returned. Target: **seconds** for a single claim on a laptop-sized store. |
| After crash mid-forget | Tombstone may exist while payload redact/blob delete is incomplete. Re-run `forget` with the same claim id; purge is idempotent. |
| Cold backups / shipped segments | **Not covered** by online forget. Operators must expire or re-forget against restored copies under their backup retention policy. Document retention separately per deployment. |
| Derived copies outside hivemind | Transcripts, agent context windows, and forks are out of band; this SLA covers the hivemind store directory only. |

Legal note: whether physical delete (or future crypto-shredding) meets a given
jurisdiction’s erasure standard is a compliance decision, not a product claim.

### Verify

```bash
hivemind recall "<distinctive secret token>" --history   # expect 0 cards
hivemind show clm_<id>                                   # expect not found
hivemind stats                                           # by_status.forgotten >= 1
hivemind verify                                          # should stay clean after purge
```

## Status framing

Ops packaging (CI gates, release artifacts, `/health` plus `/readyz` `/metrics`,
structured logs, Dockerfile) has landed. **Local MCP:** Claude Code’s default
Bash sandbox leaves MCP outside the boundary — proven wiring and
remember→recall→show smoke live in [setup-mcp.md](setup-mcp.md). Footprint:
post-mitigation RSS at 100k synthetic claims is still ~207 MB (≤ 50 MB miss);
see [README](../README.md#status-and-measurements) /
[06](06-build-plan.md). Erasure (`forget`) and its online SLA are documented
above; backup retention remains an operator policy. Full residual-risk table:
[README](../README.md#residual-risks).

