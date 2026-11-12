# Security

Threat model and secure defaults for hivemind's **shared service**
(`hivemind serve`) and notes on the local daemon path. This is not a
guarantee of fitness for every threat; it documents what the software
assumes and enforces today.

## Trust boundaries

| Surface | Trust model |
|---|---|
| Local daemon / `hivemind mcp` (stdio) | Single-user machine. Identity is trust-on-first-use via env (`HIVEMIND_PRINCIPAL` / `_AGENT` / `_SESSION`). Unix socket modes are `0700`/`0600`. |
| Shared HTTP / MCP Streamable / gRPC | Multi-tenant process. Callers present a **bootstrap bearer** or an **OAuth access token**. Write attribution uses the **token subject**, not client-supplied or env principals. |
| TLS | Process speaks **plaintext** HTTP/gRPC. Production MUST terminate TLS at a reverse proxy (or equivalent) and set `HIVEMIND_REQUIRE_TLS=1`. |

## Secrets (must be distinct)

| Secret | Env / flag | Role |
|---|---|---|
| Bootstrap bearer | `HIVEMIND_TOKEN` / `--token` | Admin/ops bearer; also accepted on protected routes. Principal: `bootstrap`. |
| OAuth client secret | `HIVEMIND_OAUTH_CLIENT_SECRET` / `--oauth-client-secret` | Confidential client at `/oauth/token`. |
| Signing key | `HIVEMIND_SIGNING_KEY` / `--signing-key` | HMAC-BLAKE3 key for sealing access tokens. |

`serve` refuses to start if any are empty or if any two are equal. Do **not**
reuse the bootstrap token as the client secret or signing key.

## Authentication and authorization

- **Open:** `/health`, `/readyz`, `/metrics`, `/healthz`, OAuth metadata,
  `/oauth/authorize`, `/oauth/token`.
- **Protected:** `/v1/tenants…`, `/mcp` — require `Authorization: Bearer …`.
- **OAuth grants:** `client_credentials`, `authorization_code` + PKCE S256, and
  `refresh_token` against one pre-registered client (`HIVEMIND_OAUTH_CLIENT_ID`).
- **Access tokens:** sealed claims (`sub`, `iss`, `exp`, …) with a **short TTL**
  (default 300s, `HIVEMIND_ACCESS_TOKEN_TTL_SECS`).
- **Refresh tokens:** opaque server-side handles issued on `authorization_code`
  (and rotated on `refresh_token`). Default TTL 7 days
  (`HIVEMIND_REFRESH_TOKEN_TTL_SECS`). Confidential client secret required for
  refresh. `client_credentials` stays access-only (re-auth with the secret).
  DCR and external IdP federation remain out of scope (P1).
- **Principal:** JSON RPC and gRPC overwrite `actor.principal` with the token
  `sub` (or `bootstrap`). Streamable MCP attributes writes to the configured
  OAuth client id. Local stdio MCP still uses env TOFU.

## Tenants

Each tenant is a separate store under `{HIVEMIND_ROOT}/tenants/{id}/`. Tenant
ids are single path components (`[A-Za-z0-9._-]`); path traversal is rejected.
There is **no** cross-tenant ACL beyond partition isolation: a valid bearer can
address any tenant id. Treat the shared service as a trusted-team boundary
unless you add an external policy layer.

## Transport (TLS contract)

1. Prefer binding to loopback or a private interface (`--listen 127.0.0.1:8787`).
2. Terminate TLS at nginx/Caddy/Envoy/cloud LB in front of the process.
3. Set `HIVEMIND_PUBLIC_BASE=https://…` so OAuth metadata advertises HTTPS.
4. Set `HIVEMIND_REQUIRE_TLS=1` (or `--require-tls`) so startup **refuses** a
   non-`https://` public base (plaintext-by-mistake guard).
5. In-process rustls listen is not required for this contract; proxies remain
   the supported production terminator.

## Threats considered

| Threat | Mitigation / residual |
|---|---|
| Stolen bootstrap token | Full admin on the shared API until rotated; keep off laptops in prod; rotate independently of signing key. |
| Stolen client secret | Can mint access tokens for the registered client until rotated. |
| Stolen signing key | Attacker can forge access tokens; rotate and invalidate by changing the key (existing tokens fail MAC). |
| Principal spoofing on shared RPC | Blocked: token `sub` overwrites request/env actor. |
| Cross-tenant read/write | Blocked at storage partition; not blocked by per-principal ACL. |
| Cleartext on the wire | `REQUIRE_TLS` + reverse-proxy contract; residual if operators skip both. |
| Token replay within TTL | Short access TTL; refresh handles rotate on use (replay of spent refresh fails). In-memory refresh store (process restart invalidates). |
| Poisoned memory as instructions | Separate trust/quarantine work (P0-C); retrieval is evidence, not instructions. |

## Reporting

Report suspected vulnerabilities privately to the repository owner. Do not file
public issues with exploit details until a fix is available.
