//! Minimal OAuth 2.1 authorization server for the shared service (M5+).
//!
//! Supports:
//! - Authorization Server Metadata ([RFC 8414](https://datatracker.ietf.org/doc/html/rfc8414))
//! - Protected Resource Metadata ([RFC 9728](https://datatracker.ietf.org/doc/html/rfc9728))
//! - `client_credentials` (machine agents)
//! - `authorization_code` + PKCE S256 (interactive / MCP clients)
//!
//! Access tokens are HMAC-BLAKE3 sealed JSON claims with a short TTL (default
//! 5 minutes). Bootstrap bearer, OAuth client secret, and signing key are
//! **separate** secrets — do not reuse the bootstrap token as the HMAC key.
//!
//! Refresh tokens, external IdP federation, and dynamic client registration
//! remain out of scope (P1).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Result, bail};
use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// Default access-token lifetime for the shared service (short TTL).
pub const DEFAULT_ACCESS_TOKEN_TTL_SECS: u64 = 300;
const CODE_TTL_SECS: u64 = 300;
/// Principal attributed to the bootstrap/admin bearer on the shared path.
pub const BOOTSTRAP_PRINCIPAL: &str = "bootstrap";

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
}

#[derive(Clone)]
pub struct OAuthState {
    pub config: OAuthConfig,
    codes: Arc<Mutex<HashMap<String, AuthCode>>>,
}

impl std::fmt::Debug for OAuthState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthState")
            .field("issuer", &self.config.issuer)
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
    pub fn new(config: OAuthConfig) -> Self {
        Self {
            config,
            codes: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn authorization_server_metadata(&self) -> Value {
        json!({
            "issuer": self.config.issuer,
            "authorization_endpoint": format!("{}/oauth/authorize", self.config.issuer),
            "token_endpoint": format!("{}/oauth/token", self.config.issuer),
            "response_types_supported": ["code"],
            "grant_types_supported": ["authorization_code", "client_credentials"],
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
        if client_id != self.config.client_id {
            bail!("unknown client_id");
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

    pub fn exchange_token(&self, form: &TokenForm) -> Result<Value> {
        let (client_id, client_secret) = resolve_client_auth(form, &self.config)?;
        match form.grant_type.as_str() {
            "client_credentials" => {
                if client_id != self.config.client_id || client_secret != self.config.client_secret
                {
                    bail!("invalid client credentials");
                }
                let token = self.issue_access_token(&client_id, form.resource.as_deref())?;
                Ok(token_response(&token, self.config.access_token_ttl_secs))
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
                // Public clients may omit secret; confidential must match.
                if !client_secret.is_empty() && client_secret != self.config.client_secret {
                    bail!("invalid client secret");
                }
                if record.redirect_uri != redirect_uri {
                    bail!("redirect_uri mismatch");
                }
                if !pkce_s256_ok(verifier, &record.code_challenge) {
                    bail!("PKCE verification failed");
                }
                let resource = form.resource.as_deref().or(record.resource.as_deref());
                let token = self.issue_access_token(&client_id, resource)?;
                Ok(token_response(&token, self.config.access_token_ttl_secs))
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

fn token_response(access_token: &str, expires_in: u64) -> Value {
    json!({
        "access_token": access_token,
        "token_type": "Bearer",
        "expires_in": expires_in,
        "scope": "hivemind",
    })
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
        })
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
        let resp = s.exchange_token(&form).unwrap();
        let id = s
            .authenticate(resp["access_token"].as_str().unwrap(), "x")
            .unwrap();
        assert_eq!(id.principal, "hivemind-cli");
    }

    #[test]
    fn require_tls_refuses_plaintext_public_base() {
        assert!(validate_transport(true, "http://127.0.0.1:8787").is_err());
        assert!(validate_transport(true, "https://memory.example").is_ok());
        assert!(validate_transport(false, "http://127.0.0.1:8787").is_ok());
    }
}
