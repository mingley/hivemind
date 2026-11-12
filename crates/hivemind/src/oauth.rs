//! Minimal OAuth 2.1 authorization server for the shared service (M5+).
//!
//! Supports:
//! - Authorization Server Metadata ([RFC 8414](https://datatracker.ietf.org/doc/html/rfc8414))
//! - Protected Resource Metadata ([RFC 9728](https://datatracker.ietf.org/doc/html/rfc9728))
//! - Dynamic Client Registration ([RFC 7591](https://datatracker.ietf.org/doc/html/rfc7591))
//! - `client_credentials` (machine agents; access token only)
//! - `authorization_code` + PKCE S256 (interactive / MCP clients; access + refresh)
//! - `refresh_token` (confidential clients; public DCR clients with `none`; rotates)
//!
//! Access tokens are HMAC-BLAKE3 sealed JSON claims with a short TTL (default
//! 5 minutes). Refresh tokens are opaque server-side handles with a longer TTL
//! (default 7 days). Bootstrap bearer, OAuth client secret, and signing key are
//! **separate** secrets — do not reuse the bootstrap token as the HMAC key.
//!
//! Dynamically registered clients are persisted under the serve data root
//! (`{HIVEMIND_ROOT}/oauth/dcr_clients.json` by default) so `client_id` /
//! `client_secret` survive process restart. Refresh-token handles remain
//! in-process. External IdP federation remains out of scope.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, bail};
use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// Default access-token lifetime for the shared service (short TTL).
pub const DEFAULT_ACCESS_TOKEN_TTL_SECS: u64 = 300;
/// Default refresh-token lifetime (interactive / long-lived MCP clients).
pub const DEFAULT_REFRESH_TOKEN_TTL_SECS: u64 = 604_800; // 7 days
const CODE_TTL_SECS: u64 = 300;
/// Principal attributed to the bootstrap/admin bearer on the shared path.
pub const BOOTSTRAP_PRINCIPAL: &str = "bootstrap";
/// Relative path under `HIVEMIND_ROOT` for the DCR client registry file.
pub const DEFAULT_OAUTH_CLIENTS_REL: &str = "oauth/dcr_clients.json";
const CLIENTS_STORE_VERSION: u32 = 1;

/// Default on-disk path for persisted DCR clients: `{root}/oauth/dcr_clients.json`.
pub fn default_clients_path(root: &Path) -> PathBuf {
    root.join(DEFAULT_OAUTH_CLIENTS_REL)
}

#[derive(Clone, Debug)]
pub struct OAuthConfig {
    /// Issuer / public base URL (no trailing slash), e.g. `https://memory.example`.
    pub issuer: String,
    pub client_id: String,
    /// Confidential-client secret (not the bootstrap bearer).
    pub client_secret: String,
    /// HMAC signing key for access tokens (not the bootstrap bearer).
    pub signing_key: String,
    /// Resource identifier advertised for MCP (usually `{issuer}/mcp`).
    pub resource: String,
    /// Access-token TTL in seconds (short by default).
    pub access_token_ttl_secs: u64,
    /// Refresh-token TTL in seconds (issued on authorization_code / refresh).
    pub refresh_token_ttl_secs: u64,
}

/// Registered OAuth client (configured bootstrap or RFC 7591 DCR).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegisteredClient {
    pub client_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_name: Option<String>,
    #[serde(default)]
    pub redirect_uris: Vec<String>,
    pub token_endpoint_auth_method: String,
    pub grant_types: Vec<String>,
    /// When true, any redirect_uri is accepted (pre-configured serve client only).
    /// Never persisted as `true` for DCR clients loaded from disk.
    #[serde(default)]
    pub open_redirect: bool,
    pub issued_at: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ClientsFile {
    version: u32,
    clients: Vec<RegisteredClient>,
}

impl RegisteredClient {
    pub fn is_confidential(&self) -> bool {
        self.token_endpoint_auth_method != "none"
    }

    pub fn allows_grant(&self, grant: &str) -> bool {
        self.grant_types.iter().any(|g| g == grant)
    }

    pub fn allows_redirect(&self, uri: &str) -> bool {
        self.open_redirect || self.redirect_uris.iter().any(|u| u == uri)
    }
}

/// Minimal RFC 7591 registration request body.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct ClientRegistrationRequest {
    #[serde(default)]
    pub redirect_uris: Vec<String>,
    #[serde(default)]
    pub client_name: Option<String>,
    #[serde(default)]
    pub token_endpoint_auth_method: Option<String>,
    #[serde(default)]
    pub grant_types: Option<Vec<String>>,
    #[serde(default)]
    pub response_types: Option<Vec<String>>,
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Clone)]
pub struct OAuthState {
    pub config: OAuthConfig,
    codes: Arc<Mutex<HashMap<String, AuthCode>>>,
    refresh: Arc<Mutex<HashMap<String, RefreshRecord>>>,
    /// Dynamically registered clients (loaded from / saved to `clients_path`).
    clients: Arc<Mutex<HashMap<String, RegisteredClient>>>,
    /// Durable DCR registry path (`None` = in-memory only, tests).
    clients_path: Option<PathBuf>,
}

impl std::fmt::Debug for OAuthState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthState")
            .field("issuer", &self.config.issuer)
            .field("clients_path", &self.clients_path)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug)]
struct AuthCode {
    client_id: String,
    redirect_uri: String,
    code_challenge: String,
    resource: Option<String>,
    exp: u64,
}

#[derive(Clone, Debug)]
struct RefreshRecord {
    client_id: String,
    sub: String,
    resource: Option<String>,
    exp: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AccessClaims {
    sub: String,
    exp: u64,
    iat: u64,
    iss: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    resource: Option<String>,
}

/// Authenticated caller derived from a bearer credential.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthIdentity {
    /// Principal used for write attribution on the shared path.
    pub principal: String,
    /// True when the shared bootstrap/admin bearer was used.
    pub bootstrap: bool,
}

impl OAuthState {
    /// In-memory OAuth state (no DCR persistence). Prefer
    /// [`Self::with_clients_path`] for `hivemind serve`.
    pub fn new(config: OAuthConfig) -> Self {
        Self::with_clients_path(config, None).expect("in-memory OAuthState cannot fail")
    }

    /// Build OAuth state, loading persisted DCR clients when `clients_path` is set.
    ///
    /// Missing files are treated as an empty registry. Corrupt / unsupported
    /// files return an error so operators notice instead of silently dropping
    /// clients. Saves use temp + rename under the same directory.
    pub fn with_clients_path(config: OAuthConfig, clients_path: Option<PathBuf>) -> Result<Self> {
        let clients = match &clients_path {
            Some(path) => load_clients(path)?,
            None => HashMap::new(),
        };
        Ok(Self {
            config,
            codes: Arc::new(Mutex::new(HashMap::new())),
            refresh: Arc::new(Mutex::new(HashMap::new())),
            clients: Arc::new(Mutex::new(clients)),
            clients_path,
        })
    }

    /// Path of the durable DCR client registry, if persistence is enabled.
    pub fn clients_path(&self) -> Option<&Path> {
        self.clients_path.as_deref()
    }

    /// Pre-configured confidential client from serve flags / env.
    fn configured_client(&self) -> RegisteredClient {
        RegisteredClient {
            client_id: self.config.client_id.clone(),
            client_secret: Some(self.config.client_secret.clone()),
            client_name: Some("hivemind configured client".into()),
            redirect_uris: Vec::new(),
            token_endpoint_auth_method: "client_secret_basic".into(),
            grant_types: vec![
                "authorization_code".into(),
                "client_credentials".into(),
                "refresh_token".into(),
            ],
            open_redirect: true,
            issued_at: 0,
        }
    }

    /// Resolve a client_id to the configured client or a DCR registration.
    pub fn lookup_client(&self, client_id: &str) -> Result<RegisteredClient> {
        if client_id == self.config.client_id {
            return Ok(self.configured_client());
        }
        let guard = self
            .clients
            .lock()
            .map_err(|_| anyhow::anyhow!("oauth client lock poisoned"))?;
        guard
            .get(client_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("unknown client_id"))
    }

    pub fn authorization_server_metadata(&self) -> Value {
        json!({
            "issuer": self.config.issuer,
            "authorization_endpoint": format!("{}/oauth/authorize", self.config.issuer),
            "token_endpoint": format!("{}/oauth/token", self.config.issuer),
            "registration_endpoint": format!("{}/oauth/register", self.config.issuer),
            "response_types_supported": ["code"],
            "grant_types_supported": [
                "authorization_code",
                "client_credentials",
                "refresh_token"
            ],
            "code_challenge_methods_supported": ["S256"],
            "token_endpoint_auth_methods_supported": [
                "client_secret_post",
                "client_secret_basic",
                "none"
            ],
            "scopes_supported": ["hivemind"],
        })
    }

    pub fn protected_resource_metadata(&self) -> Value {
        json!({
            "resource": self.config.resource,
            "authorization_servers": [self.config.issuer],
            "bearer_methods_supported": ["header"],
            "scopes_supported": ["hivemind"],
        })
    }

    pub fn mint_authorize_code(
        &self,
        client_id: &str,
        redirect_uri: &str,
        code_challenge: &str,
        code_challenge_method: &str,
        resource: Option<&str>,
    ) -> Result<String> {
        let client = self.lookup_client(client_id)?;
        if !client.allows_grant("authorization_code") {
            bail!("client not allowed authorization_code");
        }
        if !client.allows_redirect(redirect_uri) {
            bail!("redirect_uri not registered for client");
        }
        if code_challenge_method != "S256" {
            bail!("only S256 PKCE is supported");
        }
        if code_challenge.is_empty() {
            bail!("code_challenge required");
        }
        let code = random_token(24);
        let mut guard = self
            .codes
            .lock()
            .map_err(|_| anyhow::anyhow!("oauth code lock poisoned"))?;
        guard.insert(
            code.clone(),
            AuthCode {
                client_id: client_id.to_owned(),
                redirect_uri: redirect_uri.to_owned(),
                code_challenge: code_challenge.to_owned(),
                resource: resource.map(str::to_owned),
                exp: now_secs() + CODE_TTL_SECS,
            },
        );
        Ok(code)
    }

    /// RFC 7591 Dynamic Client Registration (open; rate-limited at HTTP).
    pub fn register_client(&self, req: &ClientRegistrationRequest) -> Result<Value> {
        let auth_method = req
            .token_endpoint_auth_method
            .as_deref()
            .unwrap_or("client_secret_basic");
        match auth_method {
            "client_secret_basic" | "client_secret_post" | "none" => {}
            other => bail!("unsupported token_endpoint_auth_method: {other}"),
        }

        let grant_types = req
            .grant_types
            .clone()
            .unwrap_or_else(|| vec!["authorization_code".into(), "refresh_token".into()]);
        if grant_types.is_empty() {
            bail!("grant_types required");
        }
        for g in &grant_types {
            match g.as_str() {
                "authorization_code" | "client_credentials" | "refresh_token" => {}
                other => bail!("unsupported grant_type: {other}"),
            }
        }
        if grant_types.iter().any(|g| g == "client_credentials") && auth_method == "none" {
            bail!("client_credentials requires a confidential client");
        }
        if grant_types.iter().any(|g| g == "refresh_token")
            && !grant_types.iter().any(|g| g == "authorization_code")
        {
            bail!("refresh_token requires authorization_code grant");
        }

        let needs_redirect = grant_types.iter().any(|g| g == "authorization_code");
        if needs_redirect && req.redirect_uris.is_empty() {
            bail!("redirect_uris required for authorization_code clients");
        }
        for uri in &req.redirect_uris {
            if uri.trim().is_empty() {
                bail!("redirect_uris must be non-empty");
            }
        }
        if let Some(types) = &req.response_types {
            for t in types {
                if t != "code" {
                    bail!("unsupported response_type: {t}");
                }
            }
        }

        let client_id = format!("dcr_{}", random_token(16));
        let client_secret = if auth_method == "none" {
            None
        } else {
            Some(random_token(32))
        };
        let issued_at = now_secs();
        let record = RegisteredClient {
            client_id: client_id.clone(),
            client_secret: client_secret.clone(),
            client_name: req.client_name.clone(),
            redirect_uris: req.redirect_uris.clone(),
            token_endpoint_auth_method: auth_method.to_owned(),
            grant_types: grant_types.clone(),
            open_redirect: false,
            issued_at,
        };
        {
            let mut guard = self
                .clients
                .lock()
                .map_err(|_| anyhow::anyhow!("oauth client lock poisoned"))?;
            guard.insert(client_id.clone(), record);
            if let Some(path) = &self.clients_path
                && let Err(e) = save_clients(path, &guard)
            {
                guard.remove(&client_id);
                return Err(e).context("persist DCR client registry");
            }
        }

        let mut body = json!({
            "client_id": client_id,
            "client_id_issued_at": issued_at,
            "redirect_uris": req.redirect_uris,
            "grant_types": grant_types,
            "response_types": ["code"],
            "token_endpoint_auth_method": auth_method,
            "scope": req.scope.clone().unwrap_or_else(|| "hivemind".into()),
        });
        if let Some(name) = &req.client_name {
            body["client_name"] = Value::String(name.clone());
        }
        if let Some(secret) = client_secret {
            body["client_secret"] = Value::String(secret);
            body["client_secret_expires_at"] = json!(0);
        }
        Ok(body)
    }

    pub fn exchange_token(&self, form: &TokenForm) -> Result<Value> {
        let (client_id, client_secret) = resolve_client_auth(form, &self.config)?;
        let client = self.lookup_client(&client_id)?;
        authenticate_client(&client, &client_secret)?;
        if !client.allows_grant(&form.grant_type) {
            bail!("grant_type not allowed for client");
        }
        match form.grant_type.as_str() {
            "client_credentials" => {
                // Machine clients re-authenticate with the secret; no refresh.
                let token = self.issue_access_token(&client_id, form.resource.as_deref())?;
                Ok(token_response(
                    &token,
                    self.config.access_token_ttl_secs,
                    None,
                ))
            }
            "authorization_code" => {
                let code = form
                    .code
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("code required"))?;
                let redirect_uri = form
                    .redirect_uri
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("redirect_uri required"))?;
                let verifier = form
                    .code_verifier
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("code_verifier required"))?;
                let record = {
                    let mut guard = self
                        .codes
                        .lock()
                        .map_err(|_| anyhow::anyhow!("oauth code lock poisoned"))?;
                    guard
                        .remove(code)
                        .ok_or_else(|| anyhow::anyhow!("invalid or used code"))?
                };
                if record.exp < now_secs() {
                    bail!("authorization code expired");
                }
                if record.client_id != client_id {
                    bail!("client_id mismatch");
                }
                if record.redirect_uri != redirect_uri {
                    bail!("redirect_uri mismatch");
                }
                if !pkce_s256_ok(verifier, &record.code_challenge) {
                    bail!("PKCE verification failed");
                }
                let resource = form.resource.as_deref().or(record.resource.as_deref());
                let access = self.issue_access_token(&client_id, resource)?;
                let refresh = if client.allows_grant("refresh_token") {
                    Some(self.issue_refresh_token(&client_id, &client_id, resource)?)
                } else {
                    None
                };
                Ok(token_response(
                    &access,
                    self.config.access_token_ttl_secs,
                    refresh.as_deref(),
                ))
            }
            "refresh_token" => {
                // Confidential clients must present secret (enforced above).
                // Public DCR clients (`none`) refresh with client_id only.
                let presented = form
                    .refresh_token
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("refresh_token required"))?;
                let record = {
                    let mut guard = self
                        .refresh
                        .lock()
                        .map_err(|_| anyhow::anyhow!("oauth refresh lock poisoned"))?;
                    // Rotate: consume the presented handle so replay fails.
                    guard
                        .remove(presented)
                        .ok_or_else(|| anyhow::anyhow!("invalid or revoked refresh token"))?
                };
                if record.exp < now_secs() {
                    bail!("refresh token expired");
                }
                if record.client_id != client_id {
                    bail!("client_id mismatch");
                }
                let resource = form.resource.as_deref().or(record.resource.as_deref());
                let access = self.issue_access_token(&record.sub, resource)?;
                let refresh = self.issue_refresh_token(&client_id, &record.sub, resource)?;
                Ok(token_response(
                    &access,
                    self.config.access_token_ttl_secs,
                    Some(&refresh),
                ))
            }
            other => bail!("unsupported grant_type: {other}"),
        }
    }

    fn issue_access_token(&self, sub: &str, resource: Option<&str>) -> Result<String> {
        let now = now_secs();
        let claims = AccessClaims {
            sub: sub.to_owned(),
            exp: now + self.config.access_token_ttl_secs,
            iat: now,
            iss: self.config.issuer.clone(),
            resource: resource.map(str::to_owned),
        };
        seal_token(&claims, &self.config.signing_key)
    }

    fn issue_refresh_token(
        &self,
        client_id: &str,
        sub: &str,
        resource: Option<&str>,
    ) -> Result<String> {
        let handle = random_token(32);
        let mut guard = self
            .refresh
            .lock()
            .map_err(|_| anyhow::anyhow!("oauth refresh lock poisoned"))?;
        guard.insert(
            handle.clone(),
            RefreshRecord {
                client_id: client_id.to_owned(),
                sub: sub.to_owned(),
                resource: resource.map(str::to_owned),
                exp: now_secs() + self.config.refresh_token_ttl_secs,
            },
        );
        Ok(handle)
    }

    /// Test helper: insert an already-expired refresh handle.
    #[cfg(test)]
    fn insert_expired_refresh(&self, handle: &str, client_id: &str, sub: &str) {
        let mut guard = self.refresh.lock().unwrap();
        guard.insert(
            handle.to_owned(),
            RefreshRecord {
                client_id: client_id.to_owned(),
                sub: sub.to_owned(),
                resource: None,
                exp: now_secs().saturating_sub(1),
            },
        );
    }

    /// Accept either a sealed OAuth access token or the bootstrap shared bearer.
    ///
    /// Returns the principal that must be used for write attribution on the
    /// shared path (`claims.sub` or [`BOOTSTRAP_PRINCIPAL`]).
    pub fn authenticate(&self, token: &str, bootstrap: &str) -> Result<AuthIdentity> {
        if token == bootstrap {
            return Ok(AuthIdentity {
                principal: BOOTSTRAP_PRINCIPAL.to_owned(),
                bootstrap: true,
            });
        }
        let claims = open_token(token, &self.config.signing_key)?;
        if claims.exp < now_secs() {
            bail!("access token expired");
        }
        if claims.iss != self.config.issuer {
            bail!("access token issuer mismatch");
        }
        if claims.sub.trim().is_empty() {
            bail!("access token missing subject");
        }
        Ok(AuthIdentity {
            principal: claims.sub,
            bootstrap: false,
        })
    }

    /// Back-compat wrapper: validate without returning identity.
    pub fn validate_bearer(&self, token: &str, bootstrap: &str) -> Result<()> {
        self.authenticate(token, bootstrap).map(|_| ())
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct TokenForm {
    pub grant_type: String,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub code: Option<String>,
    pub redirect_uri: Option<String>,
    pub code_verifier: Option<String>,
    pub refresh_token: Option<String>,
    pub resource: Option<String>,
    pub scope: Option<String>,
}

fn resolve_client_auth(form: &TokenForm, config: &OAuthConfig) -> Result<(String, String)> {
    let id = form
        .client_id
        .clone()
        .unwrap_or_else(|| config.client_id.clone());
    let secret = form.client_secret.clone().unwrap_or_default();
    Ok((id, secret))
}

fn authenticate_client(client: &RegisteredClient, presented_secret: &str) -> Result<()> {
    if client.is_confidential() {
        let expected = client
            .client_secret
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("client missing secret"))?;
        if presented_secret.is_empty() || presented_secret != expected {
            bail!("invalid client credentials");
        }
    } else if !presented_secret.is_empty() {
        bail!("public client must not present client_secret");
    }
    Ok(())
}

/// Parse `Authorization: Basic base64(id:secret)` into (id, secret).
pub fn parse_basic_auth(header: Option<&str>) -> Option<(String, String)> {
    let header = header?;
    let encoded = header
        .strip_prefix("Basic ")
        .or_else(|| header.strip_prefix("basic "))?;
    let bytes = STANDARD.decode(encoded.trim()).ok()?;
    let pair = String::from_utf8(bytes).ok()?;
    let (id, secret) = pair.split_once(':')?;
    Some((id.to_owned(), secret.to_owned()))
}

pub fn merge_basic_into_form(form: &mut TokenForm, basic: Option<(String, String)>) {
    if let Some((id, secret)) = basic {
        if form.client_id.is_none() {
            form.client_id = Some(id);
        }
        if form.client_secret.is_none() {
            form.client_secret = Some(secret);
        }
    }
}

fn token_response(access_token: &str, expires_in: u64, refresh_token: Option<&str>) -> Value {
    let mut body = json!({
        "access_token": access_token,
        "token_type": "Bearer",
        "expires_in": expires_in,
        "scope": "hivemind",
    });
    if let Some(refresh) = refresh_token {
        body["refresh_token"] = Value::String(refresh.to_owned());
    }
    body
}

fn seal_token(claims: &AccessClaims, key: &str) -> Result<String> {
    let payload = serde_json::to_vec(claims)?;
    let payload_b64 = URL_SAFE_NO_PAD.encode(&payload);
    let mac = blake3::keyed_hash(&key_bytes(key), payload_b64.as_bytes());
    let mac_b64 = URL_SAFE_NO_PAD.encode(mac.as_bytes());
    Ok(format!("hm1.{payload_b64}.{mac_b64}"))
}

fn open_token(token: &str, key: &str) -> Result<AccessClaims> {
    let rest = token
        .strip_prefix("hm1.")
        .ok_or_else(|| anyhow::anyhow!("not an oauth access token"))?;
    let (payload_b64, mac_b64) = rest
        .split_once('.')
        .ok_or_else(|| anyhow::anyhow!("malformed access token"))?;
    let expected = blake3::keyed_hash(&key_bytes(key), payload_b64.as_bytes());
    let got = URL_SAFE_NO_PAD
        .decode(mac_b64)
        .map_err(|_| anyhow::anyhow!("malformed access token mac"))?;
    if got.as_slice() != expected.as_bytes() {
        bail!("invalid access token signature");
    }
    let payload = URL_SAFE_NO_PAD
        .decode(payload_b64)
        .map_err(|_| anyhow::anyhow!("malformed access token payload"))?;
    Ok(serde_json::from_slice(&payload)?)
}

fn key_bytes(key: &str) -> [u8; 32] {
    *blake3::hash(key.as_bytes()).as_bytes()
}

fn pkce_s256_ok(verifier: &str, challenge: &str) -> bool {
    let digest = Sha256::digest(verifier.as_bytes());
    let computed = URL_SAFE_NO_PAD.encode(digest);
    computed == challenge
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0))
        .as_secs()
}

fn random_token(nbytes: usize) -> String {
    let mut buf = vec![0u8; nbytes];
    getrandom::fill(&mut buf).expect("getrandom");
    URL_SAFE_NO_PAD.encode(buf)
}

fn load_clients(path: &Path) -> Result<HashMap<String, RegisteredClient>> {
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let bytes = fs::read(path).with_context(|| format!("read DCR clients {}", path.display()))?;
    let file: ClientsFile = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse DCR clients {}", path.display()))?;
    if file.version != CLIENTS_STORE_VERSION {
        bail!(
            "unsupported DCR clients store version {} (expected {CLIENTS_STORE_VERSION})",
            file.version
        );
    }
    let mut map = HashMap::new();
    for mut client in file.clients {
        if client.client_id.is_empty() {
            bail!("DCR clients store contains empty client_id");
        }
        // Disk-backed entries are always DCR registrations — never open redirect.
        client.open_redirect = false;
        if map.insert(client.client_id.clone(), client).is_some() {
            bail!("DCR clients store has duplicate client_id");
        }
    }
    Ok(map)
}

fn save_clients(path: &Path, clients: &HashMap<String, RegisteredClient>) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create DCR clients dir {}", parent.display()))?;
    }
    let mut records: Vec<RegisteredClient> = clients.values().cloned().collect();
    records.sort_by(|a, b| a.client_id.cmp(&b.client_id));
    for record in &mut records {
        record.open_redirect = false;
    }
    let file = ClientsFile {
        version: CLIENTS_STORE_VERSION,
        clients: records,
    };
    let bytes = serde_json::to_vec_pretty(&file).context("serialize DCR clients")?;
    write_atomic(path, &bytes).with_context(|| format!("write DCR clients {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// Atomic replace: write `{name}.tmp` beside `path`, then rename over it.
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    {
        let mut file = File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Transport policy for the shared HTTP/gRPC listeners.
///
/// Hivemind does not terminate TLS in-process by default. Production
/// deployments MUST terminate TLS at a reverse proxy (or enable an equivalent
/// external terminator) and set `HIVEMIND_REQUIRE_TLS=1` so plaintext public
/// bases are refused at process start.
pub fn validate_transport(require_tls: bool, public_base: &str) -> Result<()> {
    let base = public_base.trim();
    if base.is_empty() {
        bail!("public base URL is required");
    }
    if require_tls {
        let lower = base.to_ascii_lowercase();
        if !lower.starts_with("https://") {
            bail!(
                "HIVEMIND_REQUIRE_TLS=1 refuses plaintext: set HIVEMIND_PUBLIC_BASE to an https:// URL (terminate TLS at the reverse proxy)"
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> OAuthState {
        OAuthState::new(OAuthConfig {
            issuer: "http://127.0.0.1:8787".into(),
            client_id: "hivemind-cli".into(),
            client_secret: "client-secret".into(),
            signing_key: "signing-key".into(),
            resource: "http://127.0.0.1:8787/mcp".into(),
            access_token_ttl_secs: DEFAULT_ACCESS_TOKEN_TTL_SECS,
            refresh_token_ttl_secs: DEFAULT_REFRESH_TOKEN_TTL_SECS,
        })
    }

    fn auth_code_form(s: &OAuthState) -> (TokenForm, String) {
        let verifier = "abcdefghijklmnopqrstuvwxyz0123456789ABCDEFG";
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let code = s
            .mint_authorize_code(
                "hivemind-cli",
                "http://127.0.0.1/cb",
                &challenge,
                "S256",
                Some("http://127.0.0.1:8787/mcp"),
            )
            .unwrap();
        let form = TokenForm {
            grant_type: "authorization_code".into(),
            client_id: Some("hivemind-cli".into()),
            client_secret: Some("client-secret".into()),
            code: Some(code),
            redirect_uri: Some("http://127.0.0.1/cb".into()),
            code_verifier: Some(verifier.into()),
            resource: Some("http://127.0.0.1:8787/mcp".into()),
            ..TokenForm::default()
        };
        (form, verifier.to_owned())
    }

    #[test]
    fn secrets_are_independent() {
        let s = state();
        let form = TokenForm {
            grant_type: "client_credentials".into(),
            client_id: Some("hivemind-cli".into()),
            client_secret: Some("client-secret".into()),
            ..TokenForm::default()
        };
        let resp = s.exchange_token(&form).unwrap();
        let token = resp["access_token"].as_str().unwrap();
        assert_eq!(resp["expires_in"], DEFAULT_ACCESS_TOKEN_TTL_SECS);
        assert!(resp.get("refresh_token").is_none());

        // Access token verifies with signing key; bootstrap is a different secret.
        let id = s.authenticate(token, "bootstrap-only").unwrap();
        assert_eq!(id.principal, "hivemind-cli");
        assert!(!id.bootstrap);

        // Bootstrap bearer is not the client secret and is not a sealed token.
        assert!(s.authenticate("client-secret", "bootstrap-only").is_err());
        let boot = s.authenticate("bootstrap-only", "bootstrap-only").unwrap();
        assert!(boot.bootstrap);
        assert_eq!(boot.principal, BOOTSTRAP_PRINCIPAL);

        // Wrong signing key cannot open the token.
        let other = OAuthState::new(OAuthConfig {
            signing_key: "other-signing-key".into(),
            ..s.config.clone()
        });
        assert!(other.authenticate(token, "bootstrap-only").is_err());
    }

    #[test]
    fn bootstrap_does_not_work_as_client_secret() {
        let s = state();
        let form = TokenForm {
            grant_type: "client_credentials".into(),
            client_id: Some("hivemind-cli".into()),
            client_secret: Some("bootstrap-only".into()),
            ..TokenForm::default()
        };
        assert!(s.exchange_token(&form).is_err());
    }

    #[test]
    fn auth_code_pkce_round_trip() {
        let s = state();
        let (form, _) = auth_code_form(&s);
        let resp = s.exchange_token(&form).unwrap();
        let id = s
            .authenticate(resp["access_token"].as_str().unwrap(), "x")
            .unwrap();
        assert_eq!(id.principal, "hivemind-cli");
        assert!(resp["refresh_token"].as_str().is_some());
    }

    #[test]
    fn refresh_token_grant_success_and_rotation() {
        let s = state();
        let (form, _) = auth_code_form(&s);
        let first = s.exchange_token(&form).unwrap();
        let refresh = first["refresh_token"].as_str().unwrap().to_owned();

        let refreshed = s
            .exchange_token(&TokenForm {
                grant_type: "refresh_token".into(),
                client_id: Some("hivemind-cli".into()),
                client_secret: Some("client-secret".into()),
                refresh_token: Some(refresh.clone()),
                ..TokenForm::default()
            })
            .unwrap();
        let new_access = refreshed["access_token"].as_str().unwrap();
        let new_refresh = refreshed["refresh_token"].as_str().unwrap();
        assert_ne!(new_refresh, refresh);
        let id = s.authenticate(new_access, "x").unwrap();
        assert_eq!(id.principal, "hivemind-cli");

        // Old refresh handle is consumed (rotation).
        let replay = s.exchange_token(&TokenForm {
            grant_type: "refresh_token".into(),
            client_id: Some("hivemind-cli".into()),
            client_secret: Some("client-secret".into()),
            refresh_token: Some(refresh),
            ..TokenForm::default()
        });
        assert!(replay.is_err());
    }

    #[test]
    fn refresh_token_rejects_invalid_and_expired() {
        let s = state();
        let invalid = s.exchange_token(&TokenForm {
            grant_type: "refresh_token".into(),
            client_id: Some("hivemind-cli".into()),
            client_secret: Some("client-secret".into()),
            refresh_token: Some("not-a-real-refresh-token".into()),
            ..TokenForm::default()
        });
        assert!(invalid.is_err());

        s.insert_expired_refresh("expired-refresh", "hivemind-cli", "hivemind-cli");
        let expired = s.exchange_token(&TokenForm {
            grant_type: "refresh_token".into(),
            client_id: Some("hivemind-cli".into()),
            client_secret: Some("client-secret".into()),
            refresh_token: Some("expired-refresh".into()),
            ..TokenForm::default()
        });
        assert!(expired.is_err());

        // Wrong confidential secret refused.
        let (form, _) = auth_code_form(&s);
        let first = s.exchange_token(&form).unwrap();
        let refresh = first["refresh_token"].as_str().unwrap().to_owned();
        let bad_secret = s.exchange_token(&TokenForm {
            grant_type: "refresh_token".into(),
            client_id: Some("hivemind-cli".into()),
            client_secret: Some("wrong-secret".into()),
            refresh_token: Some(refresh),
            ..TokenForm::default()
        });
        assert!(bad_secret.is_err());
    }

    #[test]
    fn metadata_advertises_refresh_grant() {
        let meta = state().authorization_server_metadata();
        let grants = meta["grant_types_supported"].as_array().unwrap();
        assert!(grants.iter().any(|g| g.as_str() == Some("refresh_token")));
        assert!(
            meta["registration_endpoint"]
                .as_str()
                .unwrap()
                .ends_with("/oauth/register")
        );
    }

    #[test]
    fn dcr_confidential_client_credentials() {
        let s = state();
        let reg = s
            .register_client(&ClientRegistrationRequest {
                redirect_uris: vec![],
                client_name: Some("machine".into()),
                token_endpoint_auth_method: Some("client_secret_basic".into()),
                grant_types: Some(vec!["client_credentials".into()]),
                ..ClientRegistrationRequest::default()
            })
            .unwrap();
        let client_id = reg["client_id"].as_str().unwrap();
        let client_secret = reg["client_secret"].as_str().unwrap();
        let resp = s
            .exchange_token(&TokenForm {
                grant_type: "client_credentials".into(),
                client_id: Some(client_id.into()),
                client_secret: Some(client_secret.into()),
                ..TokenForm::default()
            })
            .unwrap();
        let id = s
            .authenticate(resp["access_token"].as_str().unwrap(), "x")
            .unwrap();
        assert_eq!(id.principal, client_id);
        assert!(resp.get("refresh_token").is_none());
    }

    #[test]
    fn dcr_public_auth_code_and_refresh() {
        let s = state();
        let redirect = "http://127.0.0.1/cb";
        let reg = s
            .register_client(&ClientRegistrationRequest {
                redirect_uris: vec![redirect.into()],
                client_name: Some("mcp-public".into()),
                token_endpoint_auth_method: Some("none".into()),
                grant_types: Some(vec!["authorization_code".into(), "refresh_token".into()]),
                ..ClientRegistrationRequest::default()
            })
            .unwrap();
        let client_id = reg["client_id"].as_str().unwrap().to_owned();
        assert!(reg.get("client_secret").is_none());

        let verifier = "abcdefghijklmnopqrstuvwxyz0123456789ABCDEFG";
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let code = s
            .mint_authorize_code(&client_id, redirect, &challenge, "S256", None)
            .unwrap();
        let first = s
            .exchange_token(&TokenForm {
                grant_type: "authorization_code".into(),
                client_id: Some(client_id.clone()),
                code: Some(code),
                redirect_uri: Some(redirect.into()),
                code_verifier: Some(verifier.into()),
                ..TokenForm::default()
            })
            .unwrap();
        let refresh = first["refresh_token"].as_str().unwrap().to_owned();
        let refreshed = s
            .exchange_token(&TokenForm {
                grant_type: "refresh_token".into(),
                client_id: Some(client_id.clone()),
                refresh_token: Some(refresh),
                ..TokenForm::default()
            })
            .unwrap();
        let id = s
            .authenticate(refreshed["access_token"].as_str().unwrap(), "x")
            .unwrap();
        assert_eq!(id.principal, client_id);

        // Foreign redirect rejected at authorize.
        assert!(
            s.mint_authorize_code(&client_id, "http://evil/cb", &challenge, "S256", None)
                .is_err()
        );
    }

    #[test]
    fn dcr_rejects_public_client_credentials() {
        let s = state();
        let err = s.register_client(&ClientRegistrationRequest {
            redirect_uris: vec![],
            token_endpoint_auth_method: Some("none".into()),
            grant_types: Some(vec!["client_credentials".into()]),
            ..ClientRegistrationRequest::default()
        });
        assert!(err.is_err());
    }

    #[test]
    fn dcr_clients_persist_across_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = default_clients_path(dir.path());
        let config = OAuthConfig {
            issuer: "http://127.0.0.1:8787".into(),
            client_id: "hivemind-cli".into(),
            client_secret: "client-secret".into(),
            signing_key: "signing-key".into(),
            resource: "http://127.0.0.1:8787/mcp".into(),
            access_token_ttl_secs: DEFAULT_ACCESS_TOKEN_TTL_SECS,
            refresh_token_ttl_secs: DEFAULT_REFRESH_TOKEN_TTL_SECS,
        };
        let s = OAuthState::with_clients_path(config.clone(), Some(path.clone())).unwrap();
        let reg = s
            .register_client(&ClientRegistrationRequest {
                redirect_uris: vec![],
                client_name: Some("persist-me".into()),
                token_endpoint_auth_method: Some("client_secret_post".into()),
                grant_types: Some(vec!["client_credentials".into()]),
                ..ClientRegistrationRequest::default()
            })
            .unwrap();
        let client_id = reg["client_id"].as_str().unwrap().to_owned();
        let client_secret = reg["client_secret"].as_str().unwrap().to_owned();
        assert!(path.is_file(), "registry should be written on register");

        // Fresh process: reload from the same file.
        let reloaded = OAuthState::with_clients_path(config, Some(path)).unwrap();
        let resp = reloaded
            .exchange_token(&TokenForm {
                grant_type: "client_credentials".into(),
                client_id: Some(client_id.clone()),
                client_secret: Some(client_secret),
                ..TokenForm::default()
            })
            .unwrap();
        let id = reloaded
            .authenticate(resp["access_token"].as_str().unwrap(), "x")
            .unwrap();
        assert_eq!(id.principal, client_id);

        // Configured static confidential client still works alongside DCR.
        let configured = reloaded
            .exchange_token(&TokenForm {
                grant_type: "client_credentials".into(),
                client_id: Some("hivemind-cli".into()),
                client_secret: Some("client-secret".into()),
                ..TokenForm::default()
            })
            .unwrap();
        assert!(configured["access_token"].as_str().is_some());
    }

    #[test]
    fn require_tls_refuses_plaintext_public_base() {
        assert!(validate_transport(true, "http://127.0.0.1:8787").is_err());
        assert!(validate_transport(true, "https://memory.example").is_ok());
        assert!(validate_transport(false, "http://127.0.0.1:8787").is_ok());
    }
}
