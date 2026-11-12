# Operations

Operator guide for running hivemind as a local daemon or shared HTTP/gRPC
service. **Ops surfaces** (health probes, metrics, release packaging, container
image) are in place; this is not a claim of full product production-readiness
(no HA, no multi-region, sandbox limits still apply for local MCP).

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
| `HIVEMIND_TOKEN` | Bootstrap bearer + OAuth client secret / signing key (**required**) | — |
| `HIVEMIND_DEFAULT_TENANT` | Tenant behind `/mcp` | `default` |
| `HIVEMIND_PUBLIC_BASE` | Public base URL in OAuth metadata | `http://{listen}` |
| `HIVEMIND_OAUTH_CLIENT_ID` | Pre-registered confidential client id | `hivemind-cli` |
| `HIVEMIND_GRPC_LISTEN` | gRPC listen address | same host as HTTP, port `8788` |

Flags: `--listen` (default `127.0.0.1:8787`), `--no-grpc`, `--token`, `--root`.

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
export HIVEMIND_ROOT="$HOME/.hivemind"
cargo build --release -p hivemind
./target/release/hivemind serve --listen 127.0.0.1:8787 --root "$HIVEMIND_ROOT"
# optional JSON logs:
# HIVEMIND_LOG_FORMAT=json RUST_LOG=info ./target/release/hivemind serve ...
```

Authenticate JSON API / MCP with `Authorization: Bearer $HIVEMIND_TOKEN`, or
obtain an OAuth access token via `/oauth/token` (client credentials / auth code
+ PKCE). See M5 notes and [setup-mcp.md](setup-mcp.md) for agent wiring.

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
  -e HIVEMIND_LOG_FORMAT=json \
  -v hivemind-data:/var/lib/hivemind \
  hivemind:local
```

The image entrypoint is `hivemind serve --listen 0.0.0.0:8787 --root /var/lib/hivemind`.
Override with e.g. `docker run … hivemind:local daemon` for a single-store
daemon (publish no HTTP ports in that case; use a volume for `HIVEMIND_DIR`).

## Status framing

Ops packaging (CI gates, release artifacts, `/healthz` `/readyz` `/metrics`,
structured logs, Dockerfile) has landed. Product gaps that remain out of scope
for this doc: HA, authenticated multi-tenant isolation hardening beyond bearer
OAuth, and proving local MCP under restrictive sandboxes. Local MCP setup:
[setup-mcp.md](setup-mcp.md).
