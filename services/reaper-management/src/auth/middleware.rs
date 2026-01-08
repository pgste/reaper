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
    jwt::{Claims, JwtManager},
    scopes::{Permission, Scope},
};
use crate::state::AppState;

/// Header name for API key authentication
pub const API_KEY_HEADER: &str = "X-API-Key";

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
        // Clone what we need before moving into async block
        let api_key_header = parts.headers.get(API_KEY_HEADER).cloned();
        let auth_header = parts.headers.get(AUTHORIZATION).cloned();
        let db = state.db.clone();
        let config = state.config.clone();

        async move {
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
                    // Get JWT manager
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
                                tracing::debug!("JWT validation failed: {}", e);
                            }
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
        // Clone what we need
        let api_key_header = parts.headers.get(API_KEY_HEADER).cloned();
        let auth_header = parts.headers.get(AUTHORIZATION).cloned();
        let db = state.db.clone();
        let config = state.config.clone();

        async move {
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

#[cfg(test)]
mod tests {
    use super::*;

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
