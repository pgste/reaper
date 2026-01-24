//! OAuth integration for Git providers
//!
//! Provides OAuth flows for connecting to GitHub, GitLab, and Bitbucket
//! to enable seamless repository access for policy sources.

use axum::{
    extract::{Path, Query, State},
    http::{header::HeaderMap, StatusCode},
    response::{Json, Redirect},
    routing::{get, post},
    Router,
};
use chrono::{DateTime, Duration, Utc};
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    api::error::{ApiError, ApiResult},
    api::orgs::resolve_org,
    audit::{actions, ActorType, AuditEntry, ClientInfo, ResourceType},
    auth::{
        middleware::RequireAuth,
        users::{SessionRepository, UserOrgRepository},
    },
    db::repositories::OrganizationRepository,
    state::AppState,
};

/// Build OAuth routes
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        // GitHub OAuth
        .route("/auth/github/authorize", get(github_authorize))
        .route("/auth/github/callback", get(github_callback))
        // OAuth connections management
        .route(
            "/orgs/{org}/oauth/connections",
            get(list_connections).post(create_connection),
        )
        .route(
            "/orgs/{org}/oauth/connections/{provider}",
            get(get_connection).delete(delete_connection),
        )
        // GitHub repo listing
        .route("/orgs/{org}/github/repos", get(list_github_repos))
        .route(
            "/orgs/{org}/sources/github",
            post(create_source_from_github),
        )
}

// ==================== Types ====================

/// OAuth connection stored in database
#[derive(Debug, Clone, Serialize)]
pub struct OAuthConnection {
    pub id: Uuid,
    pub user_id: Uuid,
    pub org_id: Uuid,
    pub provider: String,
    pub provider_user_id: Option<String>,
    pub provider_username: Option<String>,
    pub token_expires_at: Option<DateTime<Utc>>,
    pub scopes: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// OAuth state token for CSRF protection
#[derive(Debug, Clone, Serialize, Deserialize)]
struct OAuthState {
    pub org_slug: String,
    pub user_id: String,
    pub nonce: String,
    pub created_at: i64,
}

impl OAuthState {
    fn new(org_slug: &str, user_id: &str) -> Self {
        let mut rng = rand::thread_rng();
        let nonce: String = (0..16).map(|_| rng.gen_range(b'a'..=b'z') as char).collect();
        Self {
            org_slug: org_slug.to_string(),
            user_id: user_id.to_string(),
            nonce,
            created_at: Utc::now().timestamp(),
        }
    }

    fn encode(&self) -> String {
        let json = serde_json::to_string(self).unwrap_or_default();
        base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, json.as_bytes())
    }

    fn decode(encoded: &str) -> Option<Self> {
        let bytes = base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, encoded).ok()?;
        let json = String::from_utf8(bytes).ok()?;
        serde_json::from_str(&json).ok()
    }

    fn is_valid(&self) -> bool {
        // State tokens expire after 10 minutes
        let now = Utc::now().timestamp();
        now - self.created_at < 600
    }
}

/// Query params for authorize endpoint
#[derive(Debug, Deserialize)]
pub struct AuthorizeParams {
    pub org: String,
}

/// Query params for callback endpoint
#[derive(Debug, Deserialize)]
pub struct CallbackParams {
    pub code: String,
    pub state: String,
}

/// GitHub access token response
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct GitHubTokenResponse {
    access_token: String,
    token_type: String,
    scope: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
}

/// GitHub user response
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct GitHubUser {
    id: i64,
    login: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    email: Option<String>,
}

/// GitHub repository
#[derive(Debug, Serialize, Deserialize)]
pub struct GitHubRepo {
    pub id: i64,
    pub name: String,
    pub full_name: String,
    pub private: bool,
    pub html_url: String,
    pub clone_url: String,
    pub ssh_url: String,
    pub default_branch: String,
    pub description: Option<String>,
}

/// Request to create a source from GitHub repo
#[derive(Debug, Deserialize)]
pub struct CreateSourceFromGitHubRequest {
    pub repo_full_name: String, // e.g., "owner/repo"
    pub branch: Option<String>,
    pub path: Option<String>,   // Path within repo to policies
    pub name: Option<String>,   // Display name for the source
}

/// List connections response
#[derive(Debug, Serialize)]
pub struct ListConnectionsResponse {
    pub connections: Vec<ConnectionSummary>,
}

/// Connection summary (without sensitive data)
#[derive(Debug, Serialize)]
pub struct ConnectionSummary {
    pub provider: String,
    pub provider_username: Option<String>,
    pub connected_at: DateTime<Utc>,
    pub scopes: Vec<String>,
}

// ==================== Handlers ====================

/// Initiate GitHub OAuth flow
async fn github_authorize(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(params): Query<AuthorizeParams>,
) -> ApiResult<Redirect> {
    // Get GitHub OAuth config
    let github_config = state
        .config
        .oauth
        .github
        .as_ref()
        .ok_or_else(|| ApiError::Internal("GitHub OAuth not configured".to_string()))?;

    // Get user from session
    let user_id = get_user_id_from_session(&state, &headers).await?;

    // Verify user has access to the org
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &params.org).await?;

    let user_org_repo = UserOrgRepository::new(&state.db);
    let role = user_org_repo
        .get_role(user_id, organization.id)
        .await?
        .ok_or_else(|| {
            ApiError::Forbidden("You are not a member of this organization".to_string())
        })?;

    if !role.can_manage_policies() {
        return Err(ApiError::Forbidden(
            "You don't have permission to connect OAuth providers".to_string(),
        ));
    }

    // Generate state token for CSRF protection
    let oauth_state = OAuthState::new(&params.org, &user_id.to_string());
    let state_token = oauth_state.encode();

    // Build GitHub authorization URL
    let auth_url = format!(
        "https://github.com/login/oauth/authorize?client_id={}&redirect_uri={}&scope=repo&state={}",
        github_config.client_id,
        urlencoding::encode(&github_config.redirect_uri),
        urlencoding::encode(&state_token)
    );

    Ok(Redirect::temporary(&auth_url))
}

/// Handle GitHub OAuth callback
async fn github_callback(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(params): Query<CallbackParams>,
) -> ApiResult<Redirect> {
    let client_info = ClientInfo::from_headers(&headers);

    // Decode and validate state token
    let oauth_state = OAuthState::decode(&params.state)
        .ok_or_else(|| ApiError::BadRequest("Invalid state token".to_string()))?;

    if !oauth_state.is_valid() {
        return Err(ApiError::BadRequest("State token expired".to_string()));
    }

    // Get GitHub OAuth config
    let github_config = state
        .config
        .oauth
        .github
        .as_ref()
        .ok_or_else(|| ApiError::Internal("GitHub OAuth not configured".to_string()))?;

    // Exchange code for access token
    let client = reqwest::Client::new();
    let token_response: GitHubTokenResponse = client
        .post("https://github.com/login/oauth/access_token")
        .header("Accept", "application/json")
        .form(&[
            ("client_id", github_config.client_id.as_str()),
            ("client_secret", github_config.client_secret.as_str()),
            ("code", &params.code),
            ("redirect_uri", &github_config.redirect_uri),
        ])
        .send()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to exchange code: {}", e)))?
        .json()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to parse token response: {}", e)))?;

    // Get GitHub user info
    let github_user: GitHubUser = client
        .get("https://api.github.com/user")
        .header("Authorization", format!("Bearer {}", token_response.access_token))
        .header("User-Agent", "Reaper-Management")
        .send()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to fetch user info: {}", e)))?
        .json()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to parse user info: {}", e)))?;

    // Get org
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &oauth_state.org_slug).await?;

    let user_id = Uuid::parse_str(&oauth_state.user_id)
        .map_err(|_| ApiError::Internal("Invalid user ID in state".to_string()))?;

    // Encrypt token before storage
    let encrypted_token = encrypt_token(&token_response.access_token, &state.config.auth.jwt_secret.clone().unwrap_or_default());
    let encrypted_refresh = token_response
        .refresh_token
        .map(|t| encrypt_token(&t, &state.config.auth.jwt_secret.clone().unwrap_or_default()));

    // Calculate token expiry
    let token_expires_at = token_response
        .expires_in
        .map(|secs| Utc::now() + Duration::seconds(secs));

    // Store OAuth connection
    let pool = state.db.sqlite_pool().ok_or(sqlx::Error::PoolClosed)?;
    let connection_id = Uuid::new_v4();
    let now = Utc::now().to_rfc3339();
    let scopes_json = serde_json::to_string(&token_response.scope.unwrap_or_default().split(',').collect::<Vec<_>>()).unwrap_or_default();

    // Insert or update connection
    sqlx::query(
        r#"
        INSERT INTO oauth_connections (
            id, user_id, org_id, provider, provider_user_id, provider_username,
            access_token_encrypted, refresh_token_encrypted, token_expires_at,
            scopes, created_at, updated_at
        ) VALUES (?, ?, ?, 'github', ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(org_id, provider) DO UPDATE SET
            user_id = excluded.user_id,
            provider_user_id = excluded.provider_user_id,
            provider_username = excluded.provider_username,
            access_token_encrypted = excluded.access_token_encrypted,
            refresh_token_encrypted = excluded.refresh_token_encrypted,
            token_expires_at = excluded.token_expires_at,
            scopes = excluded.scopes,
            updated_at = excluded.updated_at
        "#,
    )
    .bind(connection_id.to_string())
    .bind(user_id.to_string())
    .bind(organization.id.to_string())
    .bind(github_user.id.to_string())
    .bind(&github_user.login)
    .bind(&encrypted_token)
    .bind(&encrypted_refresh)
    .bind(token_expires_at.map(|t| t.to_rfc3339()))
    .bind(&scopes_json)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;

    // Audit log
    AuditEntry::builder(actions::OAUTH_CONNECT, ActorType::User, user_id.to_string())
        .org_id(organization.id)
        .resource(ResourceType::Org, organization.id.to_string())
        .ip_address(client_info.ip_address.unwrap_or_default())
        .user_agent(client_info.user_agent.unwrap_or_default())
        .details(serde_json::json!({
            "provider": "github",
            "provider_username": github_user.login
        }))
        .log(&state.db)
        .await
        .ok();

    // Redirect back to the app (frontend URL would be configured)
    Ok(Redirect::temporary(&format!(
        "/orgs/{}/settings/integrations?github=connected",
        oauth_state.org_slug
    )))
}

/// List OAuth connections for an org
async fn list_connections(
    State(state): State<Arc<AppState>>,
    RequireAuth(auth_user): RequireAuth,
    Path(org): Path<String>,
) -> ApiResult<Json<ListConnectionsResponse>> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    // Verify user has access
    let user_id = Uuid::parse_str(&auth_user.id)
        .map_err(|_| ApiError::Internal("Invalid user ID".to_string()))?;

    let user_org_repo = UserOrgRepository::new(&state.db);
    user_org_repo
        .get_role(user_id, organization.id)
        .await?
        .ok_or_else(|| {
            ApiError::Forbidden("You are not a member of this organization".to_string())
        })?;

    // Get connections
    let pool = state.db.sqlite_pool().ok_or(sqlx::Error::PoolClosed)?;
    let rows: Vec<(String, Option<String>, String, String)> = sqlx::query_as(
        "SELECT provider, provider_username, scopes, created_at FROM oauth_connections WHERE org_id = ?",
    )
    .bind(organization.id.to_string())
    .fetch_all(pool)
    .await?;

    let connections = rows
        .into_iter()
        .map(|(provider, username, scopes_json, created_at)| {
            let scopes: Vec<String> = serde_json::from_str(&scopes_json).unwrap_or_default();
            ConnectionSummary {
                provider,
                provider_username: username,
                connected_at: DateTime::parse_from_rfc3339(&created_at)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                scopes,
            }
        })
        .collect();

    Ok(Json(ListConnectionsResponse { connections }))
}

/// Get a specific OAuth connection
async fn get_connection(
    State(state): State<Arc<AppState>>,
    RequireAuth(auth_user): RequireAuth,
    Path((org, provider)): Path<(String, String)>,
) -> ApiResult<Json<ConnectionSummary>> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    // Verify user has access
    let user_id = Uuid::parse_str(&auth_user.id)
        .map_err(|_| ApiError::Internal("Invalid user ID".to_string()))?;

    let user_org_repo = UserOrgRepository::new(&state.db);
    user_org_repo
        .get_role(user_id, organization.id)
        .await?
        .ok_or_else(|| {
            ApiError::Forbidden("You are not a member of this organization".to_string())
        })?;

    // Get connection
    let pool = state.db.sqlite_pool().ok_or(sqlx::Error::PoolClosed)?;
    let row: Option<(String, Option<String>, String, String)> = sqlx::query_as(
        "SELECT provider, provider_username, scopes, created_at FROM oauth_connections WHERE org_id = ? AND provider = ?",
    )
    .bind(organization.id.to_string())
    .bind(&provider)
    .fetch_optional(pool)
    .await?;

    let (provider, username, scopes_json, created_at) = row
        .ok_or_else(|| ApiError::NotFound("OAuth connection not found".to_string()))?;

    let scopes: Vec<String> = serde_json::from_str(&scopes_json).unwrap_or_default();

    Ok(Json(ConnectionSummary {
        provider,
        provider_username: username,
        connected_at: DateTime::parse_from_rfc3339(&created_at)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now()),
        scopes,
    }))
}

/// Create OAuth connection manually (for testing/admin)
async fn create_connection(
    State(_state): State<Arc<AppState>>,
    RequireAuth(_auth_user): RequireAuth,
    Path(_org): Path<String>,
) -> ApiResult<StatusCode> {
    // This would be used for manual token entry (admin only)
    Err(ApiError::BadRequest(
        "Use OAuth flow to create connections".to_string(),
    ))
}

/// Delete OAuth connection
async fn delete_connection(
    State(state): State<Arc<AppState>>,
    RequireAuth(auth_user): RequireAuth,
    headers: HeaderMap,
    Path((org, provider)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    let client_info = ClientInfo::from_headers(&headers);

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    // Verify user has permission
    let user_id = Uuid::parse_str(&auth_user.id)
        .map_err(|_| ApiError::Internal("Invalid user ID".to_string()))?;

    let user_org_repo = UserOrgRepository::new(&state.db);
    let role = user_org_repo
        .get_role(user_id, organization.id)
        .await?
        .ok_or_else(|| {
            ApiError::Forbidden("You are not a member of this organization".to_string())
        })?;

    if !role.can_manage_policies() {
        return Err(ApiError::Forbidden(
            "You don't have permission to disconnect OAuth providers".to_string(),
        ));
    }

    // Delete connection
    let pool = state.db.sqlite_pool().ok_or(sqlx::Error::PoolClosed)?;
    let result = sqlx::query("DELETE FROM oauth_connections WHERE org_id = ? AND provider = ?")
        .bind(organization.id.to_string())
        .bind(&provider)
        .execute(pool)
        .await?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound("OAuth connection not found".to_string()));
    }

    // Audit log
    AuditEntry::builder(actions::OAUTH_DISCONNECT, ActorType::User, user_id.to_string())
        .org_id(organization.id)
        .resource(ResourceType::Org, organization.id.to_string())
        .ip_address(client_info.ip_address.unwrap_or_default())
        .user_agent(client_info.user_agent.unwrap_or_default())
        .details(serde_json::json!({
            "provider": provider
        }))
        .log(&state.db)
        .await
        .ok();

    Ok(StatusCode::NO_CONTENT)
}

/// List GitHub repositories for the connected account
async fn list_github_repos(
    State(state): State<Arc<AppState>>,
    RequireAuth(auth_user): RequireAuth,
    Path(org): Path<String>,
) -> ApiResult<Json<Vec<GitHubRepo>>> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    // Verify user has access
    let user_id = Uuid::parse_str(&auth_user.id)
        .map_err(|_| ApiError::Internal("Invalid user ID".to_string()))?;

    let user_org_repo = UserOrgRepository::new(&state.db);
    user_org_repo
        .get_role(user_id, organization.id)
        .await?
        .ok_or_else(|| {
            ApiError::Forbidden("You are not a member of this organization".to_string())
        })?;

    // Get GitHub token
    let token = get_github_token(&state, organization.id).await?;

    // Fetch repos from GitHub
    let client = reqwest::Client::new();
    let repos: Vec<GitHubRepo> = client
        .get("https://api.github.com/user/repos?per_page=100&sort=updated")
        .header("Authorization", format!("Bearer {}", token))
        .header("User-Agent", "Reaper-Management")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to fetch repos: {}", e)))?
        .json()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to parse repos: {}", e)))?;

    Ok(Json(repos))
}

/// Create a policy source from a GitHub repository
async fn create_source_from_github(
    State(state): State<Arc<AppState>>,
    RequireAuth(auth_user): RequireAuth,
    headers: HeaderMap,
    Path(org): Path<String>,
    Json(request): Json<CreateSourceFromGitHubRequest>,
) -> ApiResult<StatusCode> {
    let client_info = ClientInfo::from_headers(&headers);

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    // Verify user has permission
    let user_id = Uuid::parse_str(&auth_user.id)
        .map_err(|_| ApiError::Internal("Invalid user ID".to_string()))?;

    let user_org_repo = UserOrgRepository::new(&state.db);
    let role = user_org_repo
        .get_role(user_id, organization.id)
        .await?
        .ok_or_else(|| {
            ApiError::Forbidden("You are not a member of this organization".to_string())
        })?;

    if !role.can_manage_policies() {
        return Err(ApiError::Forbidden(
            "You don't have permission to create sources".to_string(),
        ));
    }

    // Get GitHub token for the clone URL
    let token = get_github_token(&state, organization.id).await?;

    // Build the Git clone URL with token embedded
    let clone_url = format!(
        "https://x-access-token:{}@github.com/{}.git",
        token, request.repo_full_name
    );

    // Create the Git source
    let pool = state.db.sqlite_pool().ok_or(sqlx::Error::PoolClosed)?;
    let source_id = Uuid::new_v4();
    let now = Utc::now().to_rfc3339();
    let branch = request.branch.unwrap_or_else(|| "main".to_string());
    let name = request
        .name
        .unwrap_or_else(|| request.repo_full_name.replace('/', "-"));

    let config = serde_json::json!({
        "url": clone_url,
        "branch": branch,
        "path": request.path.unwrap_or_else(|| ".".to_string()),
        "poll_interval_seconds": 300
    });

    sqlx::query(
        r#"
        INSERT INTO sources (id, org_id, namespace_id, name, source_type, config, is_active, created_at, updated_at)
        VALUES (?, ?, NULL, ?, 'git', ?, 1, ?, ?)
        "#,
    )
    .bind(source_id.to_string())
    .bind(organization.id.to_string())
    .bind(&name)
    .bind(config.to_string())
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;

    // Audit log
    AuditEntry::builder(actions::SOURCE_CREATE, ActorType::User, user_id.to_string())
        .org_id(organization.id)
        .resource(ResourceType::Source, source_id.to_string())
        .ip_address(client_info.ip_address.unwrap_or_default())
        .user_agent(client_info.user_agent.unwrap_or_default())
        .details(serde_json::json!({
            "name": name,
            "source_type": "git",
            "repo": request.repo_full_name,
            "branch": branch
        }))
        .log(&state.db)
        .await
        .ok();

    Ok(StatusCode::CREATED)
}

// ==================== Helper Functions ====================

/// Get user ID from session token in headers
async fn get_user_id_from_session(state: &AppState, headers: &HeaderMap) -> ApiResult<Uuid> {
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .ok_or_else(|| ApiError::Unauthorized("Missing or invalid Authorization header".to_string()))?;

    if !token.starts_with("rst_") {
        return Err(ApiError::Unauthorized("Invalid session token format".to_string()));
    }

    let session_repo = SessionRepository::new(&state.db);
    let session = session_repo
        .find_by_token(token)
        .await
        .map_err(|_| ApiError::Unauthorized("Invalid session".to_string()))?
        .ok_or_else(|| ApiError::Unauthorized("Invalid session".to_string()))?;

    Ok(session.user_id)
}

/// Get GitHub access token for an org
async fn get_github_token(state: &AppState, org_id: Uuid) -> ApiResult<String> {
    let pool = state.db.sqlite_pool().ok_or(sqlx::Error::PoolClosed)?;

    let row: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT access_token_encrypted, token_expires_at FROM oauth_connections WHERE org_id = ? AND provider = 'github'",
    )
    .bind(org_id.to_string())
    .fetch_optional(pool)
    .await?;

    let (encrypted_token, expires_at) = row
        .ok_or_else(|| ApiError::BadRequest("GitHub not connected. Please authorize GitHub access first.".to_string()))?;

    // Check if token is expired
    if let Some(expires_at) = expires_at {
        if let Ok(expires) = DateTime::parse_from_rfc3339(&expires_at) {
            if expires.with_timezone(&Utc) < Utc::now() {
                return Err(ApiError::BadRequest(
                    "GitHub token expired. Please reconnect GitHub.".to_string(),
                ));
            }
        }
    }

    // Decrypt token
    let token = decrypt_token(&encrypted_token, &state.config.auth.jwt_secret.clone().unwrap_or_default());

    Ok(token)
}

/// Simple token encryption (in production, use a proper encryption library)
fn encrypt_token(token: &str, key: &str) -> String {
    // This is a simple obfuscation - in production use proper encryption like AES-GCM
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    let key_hash = hasher.finalize();

    let encrypted: Vec<u8> = token
        .bytes()
        .zip(key_hash.iter().cycle())
        .map(|(t, k)| t ^ k)
        .collect();

    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &encrypted)
}

/// Simple token decryption
fn decrypt_token(encrypted: &str, key: &str) -> String {
    let encrypted_bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encrypted)
        .unwrap_or_default();

    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    let key_hash = hasher.finalize();

    let decrypted: Vec<u8> = encrypted_bytes
        .iter()
        .zip(key_hash.iter().cycle())
        .map(|(e, k)| e ^ k)
        .collect();

    String::from_utf8(decrypted).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_oauth_state_encode_decode() {
        let state = OAuthState::new("acme", "user-123");
        let encoded = state.encode();
        let decoded = OAuthState::decode(&encoded).unwrap();

        assert_eq!(decoded.org_slug, "acme");
        assert_eq!(decoded.user_id, "user-123");
        assert!(decoded.is_valid());
    }

    #[test]
    fn test_token_encryption_roundtrip() {
        let key = "test-secret-key";
        let token = "ghp_abcdefghijklmnop";

        let encrypted = encrypt_token(token, key);
        let decrypted = decrypt_token(&encrypted, key);

        assert_eq!(decrypted, token);
        assert_ne!(encrypted, token);
    }
}
