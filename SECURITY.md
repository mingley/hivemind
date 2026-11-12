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
| OAuth client secret | `HIVEMIND_OAUTH_CLIENT_SECRET` / `--oauth-client-secret` | Pre-configured confidential client at `/oauth/token`. DCR may issue additional per-client secrets. |
| Signing key | `HIVEMIND_SIGNING_KEY` / `--signing-key` | HMAC-BLAKE3 key for sealing access tokens. |

`serve` refuses to start if any are empty or if any two are equal. Do **not**
reuse the bootstrap token as the client secret or signing key.

## Authentication and authorization

- **Open:** `/health`, `/readyz`, `/metrics`, `/healthz`, OAuth metadata,
  `/oauth/authorize`, `/oauth/token`, and (unless disabled) `/oauth/register`
  (token and register are rate-limited; see below).
- **Protected:** `/v1/tenants…`, `/mcp` — require `Authorization: Bearer …`.
- **OAuth grants:** `client_credentials`, `authorization_code` + PKCE S256, and
  `refresh_token` against the pre-registered confidential client
  (`HIVEMIND_OAUTH_CLIENT_ID`) **and** dynamically registered clients (when
  DCR is enabled).
- **Authorize is auto-approve:** `/oauth/authorize` does **not** present a
  human login or consent UI. A valid `client_id` + registered `redirect_uri` +
  PKCE challenge immediately redirects with a code. Access-token `sub` is the
  OAuth **client_id** (agent/machine identity), not an end-user account. This
  matches the in-process agent AS; it is not a general IdP.
- **Dynamic Client Registration (RFC 7591):** by default `POST /oauth/register`
  is open (rate-limited) and accepts a minimal JSON body (`redirect_uris`,
  `client_name`, `token_endpoint_auth_method`, `grant_types`). Prefer
  `client_secret_basic` / `client_secret_post` (issues `client_secret`) or
  `none` (public). Default grants: `authorization_code` + `refresh_token`.
  Registered clients are persisted under
  `{HIVEMIND_ROOT}/oauth/dcr_clients.json` (override with
  `HIVEMIND_OAUTH_CLIENTS_PATH` / `--oauth-clients-path`); the file holds
  `client_secret` values and should stay mode `0600` on the host. **Kill
  switch:** `--disable-dcr` or `HIVEMIND_DISABLE_DCR=1` makes
  `/oauth/register` return 404 and omits `registration_endpoint` from AS
  metadata (pre-configured client and any clients already on disk still work).
  External IdP federation remains out of scope.
- **Access tokens:** sealed claims (`sub`, `iss`, `exp`, …) with a **short TTL**
  (default 300s, `HIVEMIND_ACCESS_TOKEN_TTL_SECS`).
- **Refresh tokens:** opaque server-side handles issued on `authorization_code`
  (and rotated on `refresh_token`). Default TTL 7 days
  (`HIVEMIND_REFRESH_TOKEN_TTL_SECS`). Handles are persisted under
  `{HIVEMIND_ROOT}/oauth/refresh_tokens.json` (override with
  `HIVEMIND_OAUTH_REFRESH_PATH` / `--oauth-refresh-path`); rewritten atomically
  on issue/rotate so interactive clients survive process restart. Confidential
  clients must present their secret; public DCR clients (`none`) refresh with
  `client_id` only. `client_credentials` stays access-only (re-auth with the
  secret).
- **Rate limits:** in-process token buckets on `/oauth/token` (per IP+`client_id`,
  default 60/min via `HIVEMIND_TOKEN_RATE_PER_MINUTE`), `/oauth/register` (per
  IP, default 10/min via `HIVEMIND_REGISTER_RATE_PER_MINUTE`), and authenticated
  HTTP/MCP/gRPC RPC (per IP+principal, default 600/min via
  `HIVEMIND_RPC_RATE_PER_MINUTE`). Excess → HTTP `429` / gRPC
  `RESOURCE_EXHAUSTED`. Ops probes are exempt. Set a knob to `0` to disable.
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
| Stolen client secret | Can mint access tokens for that client until rotated (configured or DCR). |
| Stolen signing key | Attacker can forge access tokens; rotate and invalidate by changing the key (existing tokens fail MAC). |
| Open DCR abuse | Anyone who can reach `/oauth/register` can create clients; rate-limited per IP. Residual: many source IPs / shared NAT; no admin approval step. Mitigate with network isolation and/or `--disable-dcr` / `HIVEMIND_DISABLE_DCR=1` (pre-configured client only). |
| Authorize auto-approve | No interactive user login; code issued to any registered client+redirect. Token `sub` is the client_id. Residual only if untrusted parties can reach the AS or register clients. |
| DCR client store theft | Persisted registry includes client secrets; protect `{HIVEMIND_ROOT}/oauth/` (file written `0600`). Rotate by deleting the entry / re-registering; no in-product rotation API yet. |
| Refresh token store theft | Persisted handles mint new access tokens until TTL/rotation; protect `{HIVEMIND_ROOT}/oauth/refresh_tokens.json` (`0600`). Delete the file or rotate signing/client secrets to invalidate. |
| Principal spoofing on shared RPC | Blocked: token `sub` overwrites request/env actor. |
| Cross-tenant read/write | Blocked at storage partition; not blocked by per-principal ACL. |
| Cleartext on the wire | `REQUIRE_TLS` + reverse-proxy contract; residual if operators skip both. |
| Token replay within TTL | Short access TTL; refresh handles rotate on use (replay of spent refresh fails). Spent handles are removed from the durable store on rotate. |
| Token / RPC / register abuse | Per IP+client / IP / IP+principal rate limits; residual if many source IPs or shared NAT. |
| Poisoned memory as instructions | Trust tiers + quarantine presentation (P0-C); retrieval is evidence, not instructions. |
| Session laundering of untrusted text | In-process **session taint** on `Service`: after `recall`/`show` returns untrusted or tainted claims, the reader key is marked; later `remember` from that key forces `origin=external` so the write stays tainted and `promote` still refuses below corroborated/human. **Keying:** `principal`, plus `session` when set (`principal` + U+001F + `session`). Shared path uses token `sub` / bootstrap / MCP client id as principal; local stdio uses `HIVEMIND_PRINCIPAL` (+ optional `HIVEMIND_SESSION`). Distinct sessions under one principal do not cross-contaminate. State is memory-only (cleared on process restart); F12 attack-cut measurement remains separate. |

## Reporting

Report suspected vulnerabilities privately to the repository owner. Do not file
public issues with exploit details until a fix is available.
