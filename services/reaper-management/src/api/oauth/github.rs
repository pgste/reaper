//! GitHub OAuth integration handlers

use axum::{
    extract::{Path, Query, State},
    http::{header::HeaderMap, StatusCode},
    response::{Json, Redirect},
};
use chrono::{Duration, Utc};
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    api::error::{ApiError, ApiResult},
    api::orgs::resolve_org,
    audit::{actions, ActorType, AuditEntry, ClientInfo, ResourceType},
    auth::{middleware::RequireAuth, users::UserOrgRepository},
    db::repositories::OrganizationRepository,
    state::AppState,
};

use super::helpers::{
    encrypt_token, get_github_installation_id, get_github_token, get_user_id_from_session,
};
use super::types::{
    AppSetupParams, AuthorizeParams, CallbackParams, CreateSourceFromGitHubRequest, GitHubRepo,
    GitHubTokenResponse, GitHubUser, OAuthState,
};

/// Initiate GitHub OAuth flow
#[utoipa::path(
    get,
    path = "/auth/github/authorize",
    tag = "oauth",
    responses(
        (status = 307, description = "Redirect to GitHub authorization page")
    )
)]
pub(super) async fn github_authorize(
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

    // Generate HMAC-signed state token for CSRF protection, bound to the server
    // secret so the callback cannot be tricked with a forged user_id/org_slug.
    let state_secret = state.config.auth.jwt_secret.clone().unwrap_or_default();
    let oauth_state = OAuthState::new(&params.org, &user_id.to_string());
    let state_token = oauth_state.encode(state_secret.as_bytes());

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
#[utoipa::path(
    get,
    path = "/auth/github/callback",
    tag = "oauth",
    responses(
        (status = 307, description = "Redirect back to the application after connecting")
    )
)]
pub(super) async fn github_callback(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(params): Query<CallbackParams>,
) -> ApiResult<Redirect> {
    let client_info = ClientInfo::from_headers(&headers);

    // Decode and verify the HMAC-signed state token. A forged or tampered state
    // (e.g. one naming another org/user) fails signature verification here.
    let state_secret = state.config.auth.jwt_secret.clone().unwrap_or_default();
    let oauth_state = OAuthState::decode(&params.state, state_secret.as_bytes())
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
    let client = crate::http::http_client_default()
        .map_err(|e| ApiError::Internal(format!("HTTP client build failed: {e}")))?;
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
        .header(
            "Authorization",
            format!("Bearer {}", token_response.access_token),
        )
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

    // Encrypt token before storage (authenticated encryption; fails loudly if
    // no encryption key is configured rather than storing weakly-protected data)
    let master_secret = state.config.auth.jwt_secret.clone().unwrap_or_default();
    let encrypted_token = encrypt_token(&token_response.access_token, &master_secret)?;
    let encrypted_refresh = token_response
        .refresh_token
        .map(|t| encrypt_token(&t, &master_secret))
        .transpose()?;

    // Calculate token expiry
    let token_expires_at = token_response
        .expires_in
        .map(|secs| Utc::now() + Duration::seconds(secs));

    // Store OAuth connection
    let pool = state.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;
    let connection_id = Uuid::new_v4();
    let now = Utc::now().to_rfc3339();
    let scopes_json = serde_json::to_string(
        &token_response
            .scope
            .unwrap_or_default()
            .split(',')
            .collect::<Vec<_>>(),
    )
    .unwrap_or_default();

    // Insert or update connection
    sqlx::query(
        r#"
        INSERT INTO oauth_connections (
            id, user_id, org_id, provider, provider_user_id, provider_username,
            access_token_encrypted, refresh_token_encrypted, token_expires_at,
            scopes, created_at, updated_at
        ) VALUES ($1, $2, $3, 'github', $4, $5, $6, $7, $8, $9, $10, $11)
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

/// Begin GitHub App installation (Plan 09 Step 6).
///
/// Redirects the org admin to the App's install page. GitHub then sends them
/// back to the App's configured setup URL with an `installation_id`, which
/// `github_app_setup_callback` records. This replaces the personal-OAuth-PAT
/// flow for cloning: sync auth becomes a short-lived installation token minted
/// from the App key, scoped to the repos approved at install time.
#[utoipa::path(
    get,
    path = "/orgs/{org}/git/github/install",
    tag = "oauth",
    params(("org" = String, Path, description = "Organization ID or slug")),
    responses((status = 307, description = "Redirect to the GitHub App install page"))
)]
pub(super) async fn github_app_install(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(org): Path<String>,
) -> ApiResult<Redirect> {
    let github_config = state
        .config
        .oauth
        .github
        .as_ref()
        .ok_or_else(|| ApiError::Internal("GitHub OAuth not configured".to_string()))?;
    let app_slug = github_config.app_slug.as_ref().ok_or_else(|| {
        ApiError::Internal("GitHub App not configured (missing app_slug)".to_string())
    })?;

    // AuthN + permission: only a policy manager may connect a source.
    let user_id = get_user_id_from_session(&state, &headers).await?;
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;
    let role = UserOrgRepository::new(&state.db)
        .get_role(user_id, organization.id)
        .await?
        .ok_or_else(|| {
            ApiError::Forbidden("You are not a member of this organization".to_string())
        })?;
    if !role.can_manage_policies() {
        return Err(ApiError::Forbidden(
            "You don't have permission to install the GitHub App".to_string(),
        ));
    }

    // HMAC-signed state carries the org/user through the install round-trip so
    // the setup callback can attribute the installation (CSRF-protected).
    let state_secret = state.config.auth.jwt_secret.clone().unwrap_or_default();
    let state_token = OAuthState::new(&org, &user_id.to_string()).encode(state_secret.as_bytes());

    let url = format!(
        "https://github.com/apps/{}/installations/new?state={}",
        app_slug,
        urlencoding::encode(&state_token)
    );
    Ok(Redirect::temporary(&url))
}

/// GitHub App setup callback (Plan 09 Step 6): records the `installation_id`
/// GitHub sends after an admin installs the App. Stores the installation (not
/// a user PAT), so revoking the connecting user's token never breaks sync.
#[utoipa::path(
    get,
    path = "/auth/github/app/callback",
    tag = "oauth",
    responses((status = 307, description = "Redirect back after installing the App"))
)]
pub(super) async fn github_app_setup_callback(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(params): Query<AppSetupParams>,
) -> ApiResult<Redirect> {
    let client_info = ClientInfo::from_headers(&headers);

    let state_secret = state.config.auth.jwt_secret.clone().unwrap_or_default();
    let oauth_state = OAuthState::decode(&params.state, state_secret.as_bytes())
        .ok_or_else(|| ApiError::BadRequest("Invalid state token".to_string()))?;
    if !oauth_state.is_valid() {
        return Err(ApiError::BadRequest("State token expired".to_string()));
    }

    let installation_id = params
        .installation_id
        .ok_or_else(|| ApiError::BadRequest("Missing installation_id".to_string()))?;

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &oauth_state.org_slug).await?;
    let user_id = Uuid::parse_str(&oauth_state.user_id)
        .map_err(|_| ApiError::Internal("Invalid user ID in state".to_string()))?;

    let pool = state.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        r#"
        INSERT INTO github_app_installations
            (id, org_id, installation_id, installed_by, created_at, updated_at)
        VALUES ($1, $2, $3, $4, $5, $6)
        ON CONFLICT(org_id) DO UPDATE SET
            installation_id = excluded.installation_id,
            installed_by = excluded.installed_by,
            updated_at = excluded.updated_at
        "#,
    )
    .bind(Uuid::new_v4().to_string())
    .bind(organization.id.to_string())
    .bind(&installation_id)
    .bind(user_id.to_string())
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;

    AuditEntry::builder(actions::OAUTH_CONNECT, ActorType::User, user_id.to_string())
        .org_id(organization.id)
        .resource(ResourceType::Org, organization.id.to_string())
        .ip_address(client_info.ip_address.unwrap_or_default())
        .user_agent(client_info.user_agent.unwrap_or_default())
        .details(serde_json::json!({
            "provider": "github_app",
            "installation_id": installation_id
        }))
        .log(&state.db)
        .await
        .ok();

    Ok(Redirect::temporary(&format!(
        "/orgs/{}/settings/integrations?github_app=installed",
        oauth_state.org_slug
    )))
}

/// List GitHub repositories for the connected account
#[utoipa::path(
    get,
    path = "/orgs/{org}/github/repos",
    tag = "oauth",
    params(
        ("org" = String, Path, description = "Organization ID or slug")
    ),
    responses(
        (status = 200, description = "List of GitHub repositories", body = Vec<GitHubRepo>)
    ),
    security(("bearer_jwt" = []))
)]
pub(super) async fn list_github_repos(
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
    let client = crate::http::http_client_default()
        .map_err(|e| ApiError::Internal(format!("HTTP client build failed: {e}")))?;
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
#[utoipa::path(
    post,
    path = "/orgs/{org}/sources/github",
    tag = "oauth",
    params(
        ("org" = String, Path, description = "Organization ID or slug")
    ),
    request_body = CreateSourceFromGitHubRequest,
    responses(
        (status = 201, description = "Policy source created from GitHub repository")
    ),
    security(("bearer_jwt" = []))
)]
pub(super) async fn create_source_from_github(
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

    // Require a GitHub App installation (Plan 09 Step 6 / ADR-1). The source
    // stores the installation id + repo name — NOT a token in the clone URL.
    // At sync time a short-lived installation token is minted for that one
    // clone, so revoking the connecting user's PAT never orphans the source.
    let installation_id = get_github_installation_id(&state, organization.id)
        .await?
        .ok_or_else(|| {
            ApiError::BadRequest(
                "GitHub App is not installed for this organization. Install it via \
                 /orgs/{org}/git/github/install before creating a source."
                    .to_string(),
            )
        })?;

    let branch = request.branch.unwrap_or_else(|| "main".to_string());
    let name = request
        .name
        .unwrap_or_else(|| request.repo_full_name.replace('/', "-"));

    // Token-free config: the clone URL carries no credential; provider +
    // installation_id + repo_full_name drive minted-per-sync auth.
    let config = serde_json::json!({
        "url": format!("https://github.com/{}.git", request.repo_full_name),
        "branch": branch,
        "path": request.path.unwrap_or_else(|| ".".to_string()),
        "provider": "github",
        "installation_id": installation_id,
        "repo_full_name": request.repo_full_name,
    });

    // Persist via the sources repository so the source runs through the normal
    // sync pipeline (policy_sources table, not the legacy `sources` table).
    let source_repo = crate::db::repositories::PolicySourceRepository::new(&state.db);
    let source = source_repo
        .create(
            organization.id,
            crate::domain::source::CreatePolicySource {
                name: name.clone(),
                description: Some(format!("GitHub: {}", request.repo_full_name)),
                source_type: crate::domain::source::SourceType::Git,
                config,
                sync_interval_secs: 300,
            },
        )
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to create source: {e}")))?;

    // Audit log
    AuditEntry::builder(actions::SOURCE_CREATE, ActorType::User, user_id.to_string())
        .org_id(organization.id)
        .resource(ResourceType::Source, source.id.to_string())
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
