//! Enterprise SSO — OIDC login + per-org IdP configuration (Plan 03, Phase 1).
//!
//! Native OIDC Authorization Code + PKCE. The security-critical primitives are
//! reused wholesale: the `OAuthState`-style HMAC/AEAD state via
//! [`encrypt_token`]/[`decrypt_token`], the SSRF guard
//! ([`guard_public_https_url`]) on every outbound IdP URL, and the mandatory-
//! audience ID-token validation via the existing [`JwksValidator`]. The
//! identity-table writes all funnel through [`establish_session`].

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    response::{IntoResponse, Redirect, Response},
    Json,
};
use base64::Engine;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::api::error::{ApiError, ApiResult};
use crate::api::oauth::helpers::{decrypt_token, encrypt_token};
use crate::api::orgs::{authorize_org, resolve_org};
use crate::audit::{actions, ActorType, AuditEntry};
use crate::auth::jwks::{guard_public_https_url, JwksClaims, JwksConfig};
use crate::auth::middleware::RequireAuth;
use crate::auth::scopes::Scope;
use crate::auth::sso::broker::{establish_session, ExternalIdentity, LoginContext};
use crate::auth::sso::store::SsoConfigStore;
use crate::auth::sso::{AttrMap, SsoConfig, SsoConfigInput, SsoProtocol};
use crate::auth::users::OrgRole;
use crate::auth::JwksValidator;
use crate::db::repositories::OrganizationRepository;
use crate::state::AppState;

/// State bound into the OIDC `state` parameter. Sealed with authenticated
/// encryption (`encrypt_token`) so a client cannot forge or read it: it carries
/// the org it belongs to, the ID-token `nonce`, and the PKCE verifier, making
/// the flow stateless (no server-side verifier storage).
#[derive(Debug, Serialize, Deserialize)]
struct OidcState {
    org_id: String,
    nonce: String,
    verifier: String,
    ts: i64,
}

/// OIDC discovery document (subset we use).
#[derive(Debug, Deserialize)]
struct OidcDiscovery {
    authorization_endpoint: String,
    token_endpoint: String,
    jwks_uri: String,
}

/// Token endpoint response (subset).
#[derive(Debug, Deserialize)]
struct TokenResponse {
    id_token: String,
}

/// Build the SSO routes.
pub fn routes() -> OpenApiRouter<Arc<AppState>> {
    OpenApiRouter::new()
        // Login flow (unauthenticated — this is how a user logs in).
        .routes(routes!(oidc_start))
        .routes(routes!(oidc_callback))
        // Per-org IdP configuration (org admin).
        .routes(routes!(put_sso_config, get_sso_config))
}

// ==================== Config CRUD ====================

/// Body for registering/updating an org's OIDC IdP.
#[derive(Debug, Deserialize, ToSchema)]
struct SsoConfigBody {
    #[serde(default = "default_protocol")]
    protocol: String,
    #[serde(default = "default_true")]
    enabled: bool,
    issuer: String,
    client_id: String,
    /// Plaintext client secret (encrypted before storage; never returned).
    #[serde(default)]
    client_secret: Option<String>,
    #[serde(default)]
    discovery_url: Option<String>,
    #[serde(default)]
    jwks_url: Option<String>,
    #[serde(default)]
    attr_map: Option<serde_json::Value>,
    #[serde(default)]
    allowed_domains: Option<Vec<String>>,
    #[serde(default = "default_role")]
    default_role: String,
}

fn default_protocol() -> String {
    "oidc".to_string()
}
fn default_true() -> bool {
    true
}
fn default_role() -> String {
    "viewer".to_string()
}

/// Register or update the org's SSO configuration (org admin only).
#[utoipa::path(
    put,
    path = "/orgs/{org}/sso/config",
    tag = "sso",
    params(
        ("org" = String, Path, description = "Organization ID or slug")
    ),
    request_body = SsoConfigBody,
    responses(
        (status = 200, description = "SSO configuration stored")
    ),
    security(("bearer_jwt" = []))
)]
async fn put_sso_config(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    headers: HeaderMap,
    Json(body): Json<SsoConfigBody>,
) -> ApiResult<Json<SsoConfig>> {
    let organization = authorize_org(&state, &user, &org, &[Scope::OrgAdmin]).await?;

    let protocol = SsoProtocol::parse(&body.protocol).ok_or_else(|| {
        ApiError::BadRequest(format!("unsupported SSO protocol: {}", body.protocol))
    })?;
    body.default_role.parse::<OrgRole>().map_err(|_| {
        ApiError::BadRequest(format!("invalid default_role: {}", body.default_role))
    })?;

    let secret = state.config.auth.jwt_secret.as_deref().ok_or_else(|| {
        ApiError::Internal(
            "SSO requires a configured jwt_secret to encrypt the client secret".into(),
        )
    })?;
    let client_secret_encrypted = match &body.client_secret {
        Some(s) if !s.is_empty() => Some(encrypt_token(s, secret)?),
        _ => None,
    };

    let input = SsoConfigInput {
        protocol,
        enabled: body.enabled,
        issuer: body.issuer.clone(),
        client_id: body.client_id.clone(),
        client_secret_encrypted,
        discovery_url: body.discovery_url.clone(),
        jwks_url: body.jwks_url.clone(),
        attr_map_json: body.attr_map.as_ref().map(|v| v.to_string()),
        allowed_domains_json: body
            .allowed_domains
            .as_ref()
            .map(|d| serde_json::to_string(d).unwrap_or_else(|_| "[]".into())),
        default_role: body.default_role.clone(),
    };

    let cfg = SsoConfigStore::new(&state.db)
        .upsert(organization.id, &input)
        .await?;

    let mut entry = AuditEntry::builder(
        actions::SSO_CONFIG_UPDATE,
        actor_type_of(&user),
        user.id.clone(),
    )
    .org_id(organization.id)
    .details(serde_json::json!({
        "protocol": cfg.protocol.as_str(),
        "issuer": cfg.issuer,
        "enabled": cfg.enabled,
    }));
    if let Some(ip) = client_ip(&headers) {
        entry = entry.ip_address(ip);
    }
    let _ = entry.log(&state.db).await;

    Ok(Json(cfg))
}

/// Read the org's OIDC config (secret is never serialized).
#[utoipa::path(
    get,
    path = "/orgs/{org}/sso/config",
    tag = "sso",
    params(
        ("org" = String, Path, description = "Organization ID or slug")
    ),
    responses(
        (status = 200, description = "SSO configuration (null if unset)")
    ),
    security(("bearer_jwt" = []))
)]
async fn get_sso_config(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
) -> ApiResult<Json<Option<SsoConfig>>> {
    let organization = authorize_org(&state, &user, &org, &[Scope::OrgAdmin]).await?;
    let cfg = SsoConfigStore::new(&state.db)
        .get(organization.id, SsoProtocol::Oidc)
        .await?;
    Ok(Json(cfg))
}

// ==================== OIDC login flow ====================

/// Begin OIDC login: 302 to the IdP authorize endpoint with PKCE + signed state.
#[utoipa::path(
    get,
    path = "/auth/sso/{org}/start",
    tag = "sso",
    params(
        ("org" = String, Path, description = "Organization ID or slug")
    ),
    responses(
        (status = 303, description = "Redirect to the IdP authorize endpoint")
    )
)]
async fn oidc_start(
    State(state): State<Arc<AppState>>,
    Path(org): Path<String>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let organization = resolve_org(&OrganizationRepository::new(&state.db), &org).await?;
    let cfg = SsoConfigStore::new(&state.db)
        .get_enabled_oidc(organization.id)
        .await?
        .ok_or_else(|| ApiError::NotFound("SSO is not configured for this organization".into()))?;
    let secret = sso_secret(&state)?;

    let disco = fetch_discovery(&cfg).await?;

    let verifier = gen_verifier();
    let challenge = pkce_challenge(&verifier);
    let nonce = gen_nonce();
    let redirect_uri = callback_redirect_uri(&headers, &org);

    let st = OidcState {
        org_id: organization.id.to_string(),
        nonce: nonce.clone(),
        verifier,
        ts: now_ts(),
    };
    let state_param = encrypt_token(&serde_json::to_string(&st).unwrap_or_default(), &secret)?;

    let mut url = reqwest::Url::parse(&disco.authorization_endpoint).map_err(|_| {
        ApiError::Internal("IdP returned a malformed authorization_endpoint".into())
    })?;
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &cfg.client_id)
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("scope", "openid email profile")
        .append_pair("state", &state_param)
        .append_pair("nonce", &nonce)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256");

    Ok(Redirect::to(url.as_str()).into_response())
}

#[derive(Debug, Deserialize)]
struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

/// Complete OIDC login: validate state, exchange the code, validate the ID
/// token, then establish a Reaper session.
#[utoipa::path(
    get,
    path = "/auth/sso/{org}/callback",
    tag = "sso",
    params(
        ("org" = String, Path, description = "Organization ID or slug")
    ),
    responses(
        (status = 200, description = "Session established; returns a session token")
    )
)]
async fn oidc_callback(
    State(state): State<Arc<AppState>>,
    Path(org): Path<String>,
    headers: HeaderMap,
    Query(q): Query<CallbackQuery>,
) -> ApiResult<Response> {
    if let Some(err) = q.error {
        let desc = q.error_description.unwrap_or_default();
        return Err(ApiError::Unauthorized(format!(
            "IdP returned error: {err} {desc}"
        )));
    }
    let code = q
        .code
        .ok_or_else(|| ApiError::BadRequest("missing authorization code".into()))?;
    let state_param = q
        .state
        .ok_or_else(|| ApiError::BadRequest("missing state".into()))?;

    let organization = resolve_org(&OrganizationRepository::new(&state.db), &org).await?;
    let cfg = SsoConfigStore::new(&state.db)
        .get_enabled_oidc(organization.id)
        .await?
        .ok_or_else(|| ApiError::NotFound("SSO is not configured for this organization".into()))?;
    let secret = sso_secret(&state)?;

    // Recover + validate the state (forged/tampered → decrypt fails; expiry and
    // org-binding checked explicitly).
    let st = decode_state(&state_param, &secret)?;
    validate_state(&st, &organization.id.to_string(), now_ts())?;

    let disco = fetch_discovery(&cfg).await?;
    let redirect_uri = callback_redirect_uri(&headers, &org);

    let client_secret = match &cfg.client_secret_encrypted {
        Some(enc) => Some(decrypt_token(enc, &secret)?),
        None => None,
    };
    let id_token = exchange_code(
        &disco,
        &cfg.client_id,
        client_secret.as_deref(),
        &code,
        &st.verifier,
        &redirect_uri,
    )
    .await?;

    // Validate the ID token through the shared validator (signature + iss + aud
    // + exp; rejects alg:none / HMAC confusion). Audience is the client_id.
    let jwks_url = cfg
        .jwks_url
        .clone()
        .unwrap_or_else(|| disco.jwks_uri.clone());
    let now = chrono::Utc::now();
    let jconfig = JwksConfig {
        id: cfg.id,
        org_id: organization.id,
        name: "oidc".into(),
        jwks_url,
        issuer: cfg.issuer.clone(),
        audience: Some(cfg.client_id.clone()),
        is_active: true,
        cache_ttl_secs: 300,
        created_at: now,
        updated_at: now,
    };
    let validator = state
        .jwks_validator
        .clone()
        .unwrap_or_else(|| Arc::new(JwksValidator::new()));
    let claims = validator
        .validate(&jconfig, &id_token)
        .await
        .map_err(|e| ApiError::Unauthorized(format!("ID token validation failed: {e}")))?;

    // Nonce binds this ID token to the authorize request we started.
    let token_nonce = claims.extra.get("nonce").and_then(|v| v.as_str());
    if token_nonce != Some(st.nonce.as_str()) {
        return Err(ApiError::Unauthorized("ID token nonce mismatch".into()));
    }

    let attr = cfg.attr_map();
    let email = pick_email(&claims, &attr)
        .ok_or_else(|| ApiError::BadRequest("ID token carries no email claim".into()))?;
    enforce_allowed_domains(&email, &cfg.allowed_domains())?;

    let identity = ExternalIdentity {
        issuer: claims.iss.clone(),
        subject: claims.sub.clone(),
        email,
        email_verified: pick_email_verified(&claims),
        groups: pick_groups(&claims, &attr),
        display_name: claims.name.clone(),
    };
    let ctx = LoginContext {
        ip_address: client_ip(&headers),
        user_agent: user_agent(&headers),
        session_ttl_hours: 24,
    };
    let est = establish_session(&state.db, organization.id, &identity, &cfg, &ctx).await?;

    Ok(Json(serde_json::json!({
        "session_token": est.token,
        "user_id": est.user_id,
        "role": est.role.to_string(),
        "token_type": "Bearer",
    }))
    .into_response())
}

// ==================== Pure helpers (unit-tested) ====================

fn now_ts() -> i64 {
    chrono::Utc::now().timestamp()
}

/// PKCE S256 challenge: base64url-nopad(SHA-256(verifier)).
fn pkce_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

fn gen_verifier() -> String {
    let mut bytes = [0u8; 48];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn gen_nonce() -> String {
    let mut bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn decode_state(state_param: &str, secret: &str) -> Result<OidcState, ApiError> {
    let json = decrypt_token(state_param, secret)
        .map_err(|_| ApiError::BadRequest("invalid or tampered state".into()))?;
    serde_json::from_str(&json).map_err(|_| ApiError::BadRequest("invalid state payload".into()))
}

/// State must belong to this org and be within its 10-minute lifetime.
fn validate_state(st: &OidcState, org_id: &str, now: i64) -> Result<(), ApiError> {
    if st.org_id != org_id {
        return Err(ApiError::BadRequest(
            "state does not belong to this organization".into(),
        ));
    }
    if now - st.ts > 600 {
        return Err(ApiError::BadRequest(
            "login state expired; please retry".into(),
        ));
    }
    Ok(())
}

fn pick_email(claims: &JwksClaims, attr: &AttrMap) -> Option<String> {
    // Standard `email` claim, or a configured override from `extra`.
    if attr.email_claim() == "email" {
        return claims.email.clone();
    }
    claims
        .extra
        .get(attr.email_claim())
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn pick_email_verified(claims: &JwksClaims) -> bool {
    // The IdP asserted this identity; treat email as verified unless it
    // explicitly says otherwise.
    claims
        .extra
        .get("email_verified")
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

fn pick_groups(claims: &JwksClaims, attr: &AttrMap) -> Vec<String> {
    match attr.groups_claim() {
        "groups" => claims.groups.clone(),
        "roles" => claims.roles.clone(),
        other => claims
            .extra
            .get(other)
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default(),
    }
}

fn enforce_allowed_domains(email: &str, allowed: &[String]) -> Result<(), ApiError> {
    if allowed.is_empty() {
        return Ok(());
    }
    let domain = email.rsplit('@').next().unwrap_or("").to_ascii_lowercase();
    if allowed.iter().any(|d| d == &domain) {
        Ok(())
    } else {
        Err(ApiError::Forbidden(format!(
            "email domain '{domain}' is not permitted for this organization"
        )))
    }
}

fn actor_type_of(user: &crate::auth::AuthenticatedUser) -> ActorType {
    match user.auth_method {
        crate::auth::middleware::AuthMethod::ApiKey { .. } => ActorType::ApiKey,
        crate::auth::middleware::AuthMethod::Mtls { .. } => ActorType::Agent,
        crate::auth::middleware::AuthMethod::Jwt { .. } => ActorType::User,
    }
}

fn header_str(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}

fn client_ip(headers: &HeaderMap) -> Option<String> {
    header_str(headers, "x-forwarded-for")
        .and_then(|v| v.split(',').next().map(|s| s.trim().to_string()))
}

fn user_agent(headers: &HeaderMap) -> Option<String> {
    header_str(headers, "user-agent")
}

/// The absolute callback URL (must match between authorize and token exchange).
/// Prefers an explicit `REAPER_PUBLIC_URL`; else reconstructs from the request.
fn callback_redirect_uri(headers: &HeaderMap, org: &str) -> String {
    if let Ok(base) = std::env::var("REAPER_PUBLIC_URL") {
        let base = base.trim_end_matches('/');
        return format!("{base}/auth/sso/{org}/callback");
    }
    let host = header_str(headers, "host").unwrap_or_else(|| "localhost".to_string());
    let scheme = header_str(headers, "x-forwarded-proto").unwrap_or_else(|| {
        if host.starts_with("localhost") || host.starts_with("127.0.0.1") {
            "http".to_string()
        } else {
            "https".to_string()
        }
    });
    format!("{scheme}://{host}/auth/sso/{org}/callback")
}

fn sso_secret(state: &AppState) -> Result<String, ApiError> {
    state
        .config
        .auth
        .jwt_secret
        .clone()
        .ok_or_else(|| ApiError::Internal("SSO requires a configured jwt_secret".into()))
}

// ==================== Outbound IdP calls (SSRF-guarded) ====================

fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_default()
}

async fn fetch_discovery(cfg: &SsoConfig) -> Result<OidcDiscovery, ApiError> {
    let url = cfg.effective_discovery_url();
    guard_public_https_url(&url)
        .await
        .map_err(|e| ApiError::BadRequest(format!("IdP discovery URL rejected: {e}")))?;
    let resp = http_client()
        .get(&url)
        .send()
        .await
        .map_err(|e| ApiError::Internal(format!("OIDC discovery fetch failed: {e}")))?;
    if !resp.status().is_success() {
        return Err(ApiError::Internal(format!(
            "OIDC discovery returned {}",
            resp.status()
        )));
    }
    resp.json::<OidcDiscovery>()
        .await
        .map_err(|e| ApiError::Internal(format!("OIDC discovery parse failed: {e}")))
}

async fn exchange_code(
    disco: &OidcDiscovery,
    client_id: &str,
    client_secret: Option<&str>,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<String, ApiError> {
    guard_public_https_url(&disco.token_endpoint)
        .await
        .map_err(|e| ApiError::BadRequest(format!("IdP token endpoint rejected: {e}")))?;

    let mut form = vec![
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("client_id", client_id),
        ("code_verifier", verifier),
    ];
    if let Some(secret) = client_secret {
        form.push(("client_secret", secret));
    }

    let resp = http_client()
        .post(&disco.token_endpoint)
        .form(&form)
        .send()
        .await
        .map_err(|e| ApiError::Unauthorized(format!("token exchange failed: {e}")))?;
    if !resp.status().is_success() {
        return Err(ApiError::Unauthorized(format!(
            "token endpoint returned {}",
            resp.status()
        )));
    }
    let tok = resp
        .json::<TokenResponse>()
        .await
        .map_err(|e| ApiError::Unauthorized(format!("token response parse failed: {e}")))?;
    Ok(tok.id_token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_s256_matches_rfc7636_vector() {
        // RFC 7636 Appendix B.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(
            pkce_challenge(verifier),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn state_round_trips_and_rejects_tampering() {
        let secret = "test-secret-key-for-testing-only";
        let st = OidcState {
            org_id: "org-1".into(),
            nonce: "n0nce".into(),
            verifier: "verifier-value".into(),
            ts: now_ts(),
        };
        let sealed = encrypt_token(&serde_json::to_string(&st).unwrap(), secret).unwrap();

        let decoded = decode_state(&sealed, secret).unwrap();
        assert_eq!(decoded.org_id, "org-1");
        assert_eq!(decoded.nonce, "n0nce");
        assert_eq!(decoded.verifier, "verifier-value");

        // Wrong key and tampered ciphertext both fail closed.
        assert!(decode_state(&sealed, "different-secret-value-here!").is_err());
        let mut bad = sealed.clone();
        bad.push('A');
        assert!(decode_state(&bad, secret).is_err());
    }

    #[test]
    fn state_validation_enforces_org_and_expiry() {
        let now = 1_000_000;
        let st = OidcState {
            org_id: "org-1".into(),
            nonce: "n".into(),
            verifier: "v".into(),
            ts: now,
        };
        assert!(validate_state(&st, "org-1", now + 60).is_ok());
        // Wrong org.
        assert!(validate_state(&st, "org-2", now + 60).is_err());
        // Expired (> 600s).
        assert!(validate_state(&st, "org-1", now + 601).is_err());
    }

    #[test]
    fn allowed_domains_gate() {
        let allowed = vec!["example.com".to_string()];
        assert!(enforce_allowed_domains("alice@example.com", &allowed).is_ok());
        assert!(enforce_allowed_domains("alice@EXAMPLE.com", &allowed).is_ok());
        assert!(enforce_allowed_domains("mallory@evil.com", &allowed).is_err());
        // Empty allow-list = no restriction.
        assert!(enforce_allowed_domains("anyone@anywhere.io", &[]).is_ok());
    }
}
