//! Authentication middleware and extractors
//!
//! Provides Axum extractors for authentication.

use axum::{
    extract::FromRequestParts,
    http::{header::AUTHORIZATION, request::Parts, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use std::sync::Arc;
use uuid::Uuid;

use super::{
    api_key::{ApiKey, ApiKeyRepository},
    jwks::{extract_issuer_from_token, JwksClaims, JwksConfigRepository},
    jwt::{Claims, JwtManager},
    scopes::{Permission, Scope},
    users::{SessionRepository, UserOrgRepository},
};
use crate::state::AppState;

/// Header name for API key authentication
pub const API_KEY_HEADER: &str = "X-API-Key";

/// Extract the `{org}` reference from an org-scoped request path
/// (`/orgs/{org}/…` or `/api/v1/orgs/{org}/…`), if any.
///
/// Session credentials aren't org-bound the way API keys are: a user can
/// belong to several orgs. Resolving the membership that matches the org the
/// request addresses (instead of blindly taking the first membership) is what
/// makes multi-org accounts work.
fn org_ref_from_path(path: &str) -> Option<String> {
    let mut segments = path.split('/').filter(|s| !s.is_empty());
    while let Some(segment) = segments.next() {
        if segment == "orgs" {
            return segments.next().map(|s| s.to_string());
        }
    }
    None
}

/// Pick the membership matching the org the request addresses; fall back to
/// the first membership (single-org users, non-org-scoped routes).
async fn select_membership<'m>(
    db: &crate::db::Database,
    memberships: &'m [super::users::UserOrg],
    path_org: Option<&str>,
) -> Option<&'m super::users::UserOrg> {
    if let Some(org_ref) = path_org {
        // Resolve the path reference to an org id: UUID directly, else slug.
        let org_id = match Uuid::parse_str(org_ref) {
            Ok(id) => Some(id),
            Err(_) => crate::db::repositories::OrganizationRepository::new(db)
                .get_by_slug(org_ref)
                .await
                .ok()
                .flatten()
                .map(|o| o.id),
        };
        if let Some(org_id) = org_id {
            if let Some(membership) = memberships.iter().find(|m| m.org_id == org_id) {
                return Some(membership);
            }
        }
    }
    memberships.first()
}

/// Authenticated user/agent extracted from request
#[derive(Debug, Clone)]
pub struct AuthenticatedUser {
    /// User/agent ID
    pub id: String,
    /// Organization ID
    pub org_id: Uuid,
    /// Permissions
    pub permissions: Permission,
    /// Authentication method
    pub auth_method: AuthMethod,
}

/// Authentication method used
#[derive(Debug, Clone)]
pub enum AuthMethod {
    ApiKey { key_id: Uuid },
    Jwt { token_id: String },
    Mtls { cert_id: Uuid },
}

impl AuthenticatedUser {
    /// Create from API key
    pub fn from_api_key(api_key: &ApiKey) -> Self {
        Self {
            id: api_key.id.to_string(),
            org_id: api_key.org_id,
            permissions: Permission::from_strings(&api_key.scopes),
            auth_method: AuthMethod::ApiKey { key_id: api_key.id },
        }
    }

    /// Create from a validated client certificate (mTLS).
    ///
    /// The certificate has already passed [`crate::auth::mtls::validate_certificate`]
    /// (registered, not revoked, within validity, agent binding checked), so it
    /// is granted the standard agent scopes for its organization.
    pub fn from_certificate(cert: &crate::auth::mtls::ClientCertificate) -> Self {
        let scopes: Vec<String> = super::scopes::Scope::agent_defaults()
            .iter()
            .map(|s| s.to_string())
            .collect();
        Self {
            id: cert
                .agent_id
                .map(|a| a.to_string())
                .unwrap_or_else(|| cert.id.to_string()),
            org_id: cert.org_id,
            permissions: Permission::from_strings(&scopes),
            auth_method: AuthMethod::Mtls { cert_id: cert.id },
        }
    }

    /// Create from JWT claims
    pub fn from_claims(claims: &Claims) -> Option<Self> {
        let org_id = claims.org_uuid()?;
        Some(Self {
            id: claims.sub.clone(),
            org_id,
            permissions: Permission::from_strings(&claims.scopes),
            auth_method: AuthMethod::Jwt {
                token_id: claims.jti.clone(),
            },
        })
    }

    /// Create from JWKS-validated claims
    ///
    /// The org_id is provided by the JWKS configuration (not from the token),
    /// since the configuration is scoped to an organization.
    pub fn from_jwks_claims(claims: &JwksClaims, org_id: Uuid) -> Self {
        // Combine groups and roles for permission mapping
        let mut scopes: Vec<String> = claims.groups.clone();
        scopes.extend(claims.roles.clone());

        Self {
            id: claims.sub.clone(),
            org_id,
            permissions: Permission::from_strings(&scopes),
            auth_method: AuthMethod::Jwt {
                token_id: claims.jti.clone().unwrap_or_default(),
            },
        }
    }

    /// Check if user has a specific permission
    pub fn has_permission(&self, scope: Scope) -> bool {
        self.permissions.has(scope)
    }

    /// Check if user has any of the specified permissions
    pub fn has_any_permission(&self, scopes: &[Scope]) -> bool {
        self.permissions.has_any(scopes)
    }
}

/// Authentication error response
#[derive(Debug, Serialize)]
struct AuthError {
    error: String,
    message: String,
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        (StatusCode::UNAUTHORIZED, Json(self)).into_response()
    }
}

/// Extractor that requires authentication (API key or JWT)
#[derive(Debug, Clone)]
pub struct RequireAuth(pub AuthenticatedUser);

impl FromRequestParts<Arc<AppState>> for RequireAuth {
    type Rejection = Response;

    fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send {
        // Fast path: the default-deny `require_authentication` gateway already
        // authenticated this request and stashed the user in extensions. Reuse
        // it so the handler extractor doesn't re-run DB/JWT validation.
        let cached = parts.extensions.get::<AuthenticatedUser>().cloned();
        // Clone what we need before moving into async block
        let api_key_header = parts.headers.get(API_KEY_HEADER).cloned();
        let auth_header = parts.headers.get(AUTHORIZATION).cloned();
        let path_org = org_ref_from_path(parts.uri.path());
        let db = state.db.clone();
        let config = state.config.clone();
        let jwks_validator = state.jwks_validator.clone();

        // mTLS client-cert fingerprint, only when a trusted-proxy header name is
        // configured (mTLS auth is disabled by default).
        let mtls_fingerprint = config
            .auth
            .mtls_fingerprint_header
            .as_ref()
            .and_then(|name| parts.headers.get(name.as_str()))
            .and_then(|v| v.to_str().ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        async move {
            if let Some(user) = cached {
                return Ok(RequireAuth(user));
            }
            // Try API key first
            if let Some(api_key_value) = api_key_header {
                let api_key_str = api_key_value.to_str().map_err(|_| {
                    AuthError {
                        error: "invalid_header".to_string(),
                        message: "Invalid API key header".to_string(),
                    }
                    .into_response()
                })?;

                let repo = ApiKeyRepository::new(&db);
                if let Some(api_key) = repo.validate(api_key_str).await.map_err(|e| {
                    tracing::error!("API key validation error: {}", e);
                    AuthError {
                        error: "auth_error".to_string(),
                        message: "Authentication error".to_string(),
                    }
                    .into_response()
                })? {
                    return Ok(RequireAuth(AuthenticatedUser::from_api_key(&api_key)));
                }
            }

            // Try mTLS client certificate (only when configured). The certificate
            // is checked against the DB for registration, revocation, validity
            // window, and agent binding — so revoking a cert immediately denies it.
            if let Some(fingerprint) = mtls_fingerprint {
                match crate::auth::mtls::validate_certificate(&db, &fingerprint, None).await {
                    Ok(cert) => {
                        return Ok(RequireAuth(AuthenticatedUser::from_certificate(&cert)));
                    }
                    Err(e) => {
                        tracing::warn!("mTLS certificate rejected: {}", e);
                        return Err(AuthError {
                            error: "invalid_certificate".to_string(),
                            message: "Client certificate is not valid".to_string(),
                        }
                        .into_response());
                    }
                }
            }

            // Try JWT bearer token
            if let Some(auth_header_value) = auth_header {
                let auth_str = auth_header_value.to_str().map_err(|_| {
                    AuthError {
                        error: "invalid_header".to_string(),
                        message: "Invalid Authorization header".to_string(),
                    }
                    .into_response()
                })?;

                if let Some(token) = auth_str.strip_prefix("Bearer ") {
                    // First, try session token (rst_ prefix for user sessions)
                    if token.starts_with("rst_") {
                        let session_repo = SessionRepository::new(&db);
                        match session_repo.find_by_token(token).await {
                            Ok(Some(session)) => {
                                // Resolve the membership matching the org this
                                // request addresses (multi-org users), falling
                                // back to the first membership.
                                let user_org_repo = UserOrgRepository::new(&db);
                                if let Ok(memberships) =
                                    user_org_repo.get_user_orgs(session.user_id).await
                                {
                                    if let Some(membership) =
                                        select_membership(&db, &memberships, path_org.as_deref())
                                            .await
                                    {
                                        // Convert org role to scopes
                                        let scopes = role_to_scopes(membership.role);
                                        return Ok(RequireAuth(AuthenticatedUser {
                                            id: session.user_id.to_string(),
                                            org_id: membership.org_id,
                                            permissions: Permission::from_strings(&scopes),
                                            auth_method: AuthMethod::Jwt {
                                                token_id: session.id.to_string(),
                                            },
                                        }));
                                    }
                                }
                                // User has no org memberships - still authenticated but limited
                                return Err(AuthError {
                                    error: "no_org".to_string(),
                                    message: "User has no organization memberships".to_string(),
                                }
                                .into_response());
                            }
                            Ok(None) => {
                                tracing::debug!("Session token not found");
                            }
                            Err(e) => {
                                tracing::debug!("Session validation failed: {}", e);
                            }
                        }
                    }

                    // Second, try shared-secret JWT (internal tokens from agent registration)
                    if let Some(ref jwt_secret) = config.auth.jwt_secret {
                        let manager = JwtManager::with_secret(
                            jwt_secret,
                            &config.auth.jwt_issuer,
                            &config.auth.jwt_audience,
                            config.auth.jwt_expiry_hours,
                        );

                        match manager.validate(token) {
                            Ok(claims) => {
                                if let Some(user) = AuthenticatedUser::from_claims(&claims) {
                                    return Ok(RequireAuth(user));
                                }
                            }
                            Err(e) => {
                                tracing::debug!("Shared-secret JWT validation failed: {}", e);
                            }
                        }
                    }

                    // Second, try JWKS validation (external IdP tokens)
                    if let Some(ref validator) = jwks_validator {
                        // Extract issuer from token to find the right JWKS config
                        if let Some(issuer) = extract_issuer_from_token(token) {
                            let repo = JwksConfigRepository::new(&db);

                            // Find JWKS configs matching this issuer
                            match repo.find_by_issuer(&issuer).await {
                                Ok(configs) => {
                                    // Try each matching config until one succeeds
                                    for jwks_config in configs {
                                        match validator.validate(&jwks_config, token).await {
                                            Ok(claims) => {
                                                tracing::debug!(
                                                    issuer = %issuer,
                                                    org_id = %jwks_config.org_id,
                                                    subject = %claims.sub,
                                                    "JWKS authentication successful"
                                                );
                                                return Ok(RequireAuth(
                                                    AuthenticatedUser::from_jwks_claims(
                                                        &claims,
                                                        jwks_config.org_id,
                                                    ),
                                                ));
                                            }
                                            Err(e) => {
                                                tracing::debug!(
                                                    issuer = %issuer,
                                                    config_id = %jwks_config.id,
                                                    error = %e,
                                                    "JWKS validation failed, trying next config"
                                                );
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    tracing::debug!(
                                        issuer = %issuer,
                                        error = %e,
                                        "Failed to look up JWKS configs by issuer"
                                    );
                                }
                            }
                        } else {
                            tracing::debug!(
                                "Could not extract issuer from token for JWKS validation"
                            );
                        }
                    }
                }
            }

            Err(AuthError {
                error: "unauthorized".to_string(),
                message: "Authentication required. Provide X-API-Key header or Bearer token."
                    .to_string(),
            }
            .into_response())
        }
    }
}

/// Extractor that optionally extracts authentication
#[derive(Debug, Clone)]
pub struct OptionalAuth(pub Option<AuthenticatedUser>);

impl FromRequestParts<Arc<AppState>> for OptionalAuth {
    type Rejection = Response;

    fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send {
        // Fast path: reuse the gateway's resolved user (same request URI, so
        // org-aware session selection already happened there).
        let cached = parts.extensions.get::<AuthenticatedUser>().cloned();
        // Clone what we need
        let api_key_header = parts.headers.get(API_KEY_HEADER).cloned();
        let auth_header = parts.headers.get(AUTHORIZATION).cloned();
        let path_org = org_ref_from_path(parts.uri.path());
        let db = state.db.clone();
        let config = state.config.clone();
        let jwks_validator = state.jwks_validator.clone();

        async move {
            if cached.is_some() {
                return Ok(OptionalAuth(cached));
            }
            // Try API key first
            if let Some(api_key_value) = api_key_header {
                if let Ok(api_key_str) = api_key_value.to_str() {
                    let repo = ApiKeyRepository::new(&db);
                    if let Ok(Some(api_key)) = repo.validate(api_key_str).await {
                        return Ok(OptionalAuth(Some(AuthenticatedUser::from_api_key(
                            &api_key,
                        ))));
                    }
                }
            }

            // Try JWT bearer token
            if let Some(auth_header_value) = auth_header {
                if let Ok(auth_str) = auth_header_value.to_str() {
                    if let Some(token) = auth_str.strip_prefix("Bearer ") {
                        // First, try session token (rst_ prefix)
                        if token.starts_with("rst_") {
                            let session_repo = SessionRepository::new(&db);
                            if let Ok(Some(session)) = session_repo.find_by_token(token).await {
                                let user_org_repo = UserOrgRepository::new(&db);
                                if let Ok(memberships) =
                                    user_org_repo.get_user_orgs(session.user_id).await
                                {
                                    if let Some(membership) =
                                        select_membership(&db, &memberships, path_org.as_deref())
                                            .await
                                    {
                                        let scopes = role_to_scopes(membership.role);
                                        return Ok(OptionalAuth(Some(AuthenticatedUser {
                                            id: session.user_id.to_string(),
                                            org_id: membership.org_id,
                                            permissions: Permission::from_strings(&scopes),
                                            auth_method: AuthMethod::Jwt {
                                                token_id: session.id.to_string(),
                                            },
                                        })));
                                    }
                                }
                            }
                        }

                        // Second, try shared-secret JWT
                        if let Some(ref jwt_secret) = config.auth.jwt_secret {
                            let manager = JwtManager::with_secret(
                                jwt_secret,
                                &config.auth.jwt_issuer,
                                &config.auth.jwt_audience,
                                config.auth.jwt_expiry_hours,
                            );

                            if let Ok(claims) = manager.validate(token) {
                                if let Some(user) = AuthenticatedUser::from_claims(&claims) {
                                    return Ok(OptionalAuth(Some(user)));
                                }
                            }
                        }

                        // Third, try JWKS validation
                        if let Some(ref validator) = jwks_validator {
                            if let Some(issuer) = extract_issuer_from_token(token) {
                                let repo = JwksConfigRepository::new(&db);
                                if let Ok(configs) = repo.find_by_issuer(&issuer).await {
                                    for jwks_config in configs {
                                        if let Ok(claims) =
                                            validator.validate(&jwks_config, token).await
                                        {
                                            return Ok(OptionalAuth(Some(
                                                AuthenticatedUser::from_jwks_claims(
                                                    &claims,
                                                    jwks_config.org_id,
                                                ),
                                            )));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            Ok(OptionalAuth(None))
        }
    }
}

/// Require specific scope - helper for checking permissions in handlers
pub struct RequireScope;

impl RequireScope {
    /// Check if authenticated user has the required scope
    #[allow(clippy::result_large_err)]
    pub fn check(user: &AuthenticatedUser, scope: Scope) -> Result<(), Response> {
        if user.has_permission(scope) {
            return Ok(());
        }
        Err(AuthError {
            error: "forbidden".to_string(),
            message: format!("Missing required scope: {}", scope),
        }
        .into_response())
    }
}

/// Convert an OrgRole to a list of permission scope strings
fn role_to_scopes(role: super::users::OrgRole) -> Vec<String> {
    use super::users::OrgRole;
    match role {
        // An org Owner has FULL control of their OWN organization, but is NOT a
        // platform super-admin. The global "admin" scope is deliberately not
        // granted here: cross-organization guards use `!has(Scope::Admin)` as
        // their escape hatch, so granting "admin" to every Owner (and every
        // org is created with an Owner) made every tenant able to act on every
        // other tenant's resources. "admin" is reserved for genuine
        // platform operators and is never conferred by an org role.
        OrgRole::Owner => vec![
            "org:admin".to_string(),
            "org:read".to_string(),
            "org:write".to_string(),
            "agent:register".to_string(),
            "agent:read".to_string(),
            "agent:write".to_string(),
            "policy:read".to_string(),
            "policy:write".to_string(),
            "bundle:read".to_string(),
            "bundle:write".to_string(),
            "bundle:promote".to_string(),
            // Owner has full control, so may both request and approve. Real
            // separation of duties for a change-approval board is achieved by
            // granting `bundle:approve` to a dedicated IdP group / API key
            // *without* `bundle:promote`, not by relying on the Owner role.
            "bundle:approve".to_string(),
            "apikey:read".to_string(),
            "apikey:write".to_string(),
        ],
        // Org Admin can manage bundles but deliberately cannot *originate* a
        // promotion (no `bundle:promote`); giving it `bundle:approve` makes it
        // the built-in approver role — a clean separation from the Developers /
        // Owner who stage and request a change.
        OrgRole::Admin => vec![
            "org:admin".to_string(),
            "agent:read".to_string(),
            "agent:write".to_string(),
            "policy:read".to_string(),
            "policy:write".to_string(),
            "bundle:read".to_string(),
            "bundle:write".to_string(),
            "bundle:approve".to_string(),
            "apikey:read".to_string(),
            "apikey:write".to_string(),
        ],
        OrgRole::Developer => vec![
            "agent:read".to_string(),
            "agent:write".to_string(),
            "policy:read".to_string(),
            "policy:write".to_string(),
            "bundle:read".to_string(),
            "bundle:write".to_string(),
        ],
        OrgRole::Viewer => vec![
            "agent:read".to_string(),
            "policy:read".to_string(),
            "bundle:read".to_string(),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_owner_is_not_platform_admin() {
        // Regression for the tenant-isolation break: an org Owner must have full
        // control of their OWN org but must NOT hold the global platform-admin
        // scope, which cross-org guards use as their escape hatch.
        let scopes = role_to_scopes(super::super::users::OrgRole::Owner);
        assert!(
            !scopes.contains(&"admin".to_string()),
            "Owner must not be granted the global platform-admin scope"
        );

        let perm = Permission::from_strings(&scopes);
        assert!(
            !perm.has(Scope::Admin),
            "Owner is not a platform super-admin"
        );
        assert!(perm.has(Scope::OrgAdmin), "Owner is a full org admin");
        assert!(perm.has(Scope::PolicyWrite));
        assert!(perm.has(Scope::ApiKeyWrite));
        assert!(perm.has(Scope::BundlePromote));
    }

    #[test]
    fn test_authenticated_user_from_api_key() {
        let api_key = ApiKey {
            id: Uuid::new_v4(),
            org_id: Uuid::new_v4(),
            name: "Test".to_string(),
            key_prefix: "rpr_12345678".to_string(),
            scopes: vec!["agent:read".to_string(), "policy:read".to_string()],
            expires_at: None,
            last_used_at: None,
            is_revoked: false,
            created_at: chrono::Utc::now(),
            created_by: None,
        };

        let user = AuthenticatedUser::from_api_key(&api_key);
        assert_eq!(user.id, api_key.id.to_string());
        assert!(user.has_permission(Scope::AgentRead));
        assert!(user.has_permission(Scope::PolicyRead));
        assert!(!user.has_permission(Scope::PolicyWrite));
    }

    #[test]
    fn test_authenticated_user_from_claims() {
        let org_id = Uuid::new_v4();
        let claims = Claims {
            sub: "agent-123".to_string(),
            iss: "reaper".to_string(),
            aud: "reaper-agent".to_string(),
            exp: chrono::Utc::now().timestamp() + 3600,
            iat: chrono::Utc::now().timestamp(),
            jti: Uuid::new_v4().to_string(),
            org_id: org_id.to_string(),
            scopes: vec!["admin".to_string()],
            custom: serde_json::json!({}),
        };

        let user = AuthenticatedUser::from_claims(&claims).unwrap();
        assert_eq!(user.id, "agent-123");
        assert_eq!(user.org_id, org_id);
        // Admin has all permissions
        assert!(user.has_permission(Scope::AgentRead));
        assert!(user.has_permission(Scope::PolicyWrite));
    }
}
