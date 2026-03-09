//! User authentication handlers.

use axum::{
    extract::State,
    http::{header::HeaderMap, StatusCode},
    response::Json,
};
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    api::error::{ApiError, ApiResult},
    audit::{actions, ActorType, AuditEntry, ClientInfo, ResourceType},
    auth::users::{
        hash_password, OrgRole, PasswordResetRepository, PasswordResetToken, Session,
        SessionRepository, User, UserError, UserOrg, UserOrgRepository, UserRepository,
    },
    db::repositories::OrganizationRepository,
    domain::CreateOrganization,
    state::AppState,
};

use super::helpers::{
    get_session_token, get_user_from_session, is_valid_email, is_valid_slug, slugify,
    validate_password,
};
use super::types::{
    ChangePasswordRequest, LoginRequest, LoginResponse, OrgInfo, OrgMembership,
    RequestPasswordResetRequest, ResetPasswordRequest, SignupRequest, SignupResponse, UserInfo,
};

/// Sign up a new user and create their first organization
pub async fn signup(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<SignupRequest>,
) -> ApiResult<(StatusCode, Json<SignupResponse>)> {
    let client_info = ClientInfo::from_headers(&headers);

    // Validate email format
    if !is_valid_email(&request.email) {
        return Err(ApiError::BadRequest("Invalid email format".to_string()));
    }

    // Validate password strength
    if let Err(msg) = validate_password(&request.password) {
        return Err(ApiError::BadRequest(msg));
    }

    // Generate org slug if not provided
    let org_slug = request
        .org_slug
        .unwrap_or_else(|| slugify(&request.org_name));

    // Validate org slug
    if !is_valid_slug(&org_slug) {
        return Err(ApiError::BadRequest(
            "Invalid org slug. Use only lowercase letters, numbers, and hyphens.".to_string(),
        ));
    }

    // Check if email already exists
    let user_repo = UserRepository::new(&state.db);
    if user_repo.find_by_email(&request.email).await?.is_some() {
        return Err(ApiError::Conflict("Email already registered".to_string()));
    }

    // Check if org slug already exists
    let org_repo = OrganizationRepository::new(&state.db);
    if org_repo.get_by_slug(&org_slug).await?.is_some() {
        return Err(ApiError::Conflict(
            "Organization slug already taken".to_string(),
        ));
    }

    // Create user
    let user = User::new(request.email.clone(), &request.password)
        .map_err(|e| ApiError::Internal(format!("Failed to create user: {}", e)))?;
    user_repo.create(&user).await.map_err(|e| match e {
        UserError::EmailExists => ApiError::Conflict("Email already registered".to_string()),
        other => ApiError::Internal(format!("Failed to create user: {}", other)),
    })?;

    // Create organization
    let create_org = CreateOrganization {
        name: request.org_name.clone(),
        slug: org_slug.clone(),
        display_name: None,
        description: None,
        settings: serde_json::json!({}),
    };
    let org = org_repo.create(create_org).await?;

    // Create user-org membership as owner
    let user_org_repo = UserOrgRepository::new(&state.db);
    let membership = UserOrg {
        id: Uuid::new_v4(),
        user_id: user.id,
        org_id: org.id,
        role: OrgRole::Owner,
        invited_by: None,
        joined_at: chrono::Utc::now(),
    };
    user_org_repo.add_membership(&membership).await?;

    // Create session
    let session_repo = SessionRepository::new(&state.db);
    let (session, token) = Session::new(
        user.id,
        client_info.ip_address.clone(),
        client_info.user_agent.clone(),
        state.config.auth.jwt_expiry_hours,
    );
    session_repo.create(&session).await?;

    // Extract for audit logs (used twice)
    let ip = client_info.ip_address.unwrap_or_default();
    let ua = client_info.user_agent.unwrap_or_default();

    // Audit log: user signup
    AuditEntry::builder(actions::USER_SIGNUP, ActorType::User, user.id.to_string())
        .org_id(org.id)
        .resource(ResourceType::User, user.id.to_string())
        .ip_address(ip.clone())
        .user_agent(ua.clone())
        .details(serde_json::json!({
            "email": user.email,
            "org_name": org.name,
            "org_slug": org.slug
        }))
        .log(&state.db)
        .await
        .ok();

    // Audit log: org creation
    AuditEntry::builder(actions::ORG_CREATE, ActorType::User, user.id.to_string())
        .org_id(org.id)
        .resource(ResourceType::Org, org.id.to_string())
        .ip_address(ip)
        .user_agent(ua)
        .details(serde_json::json!({
            "name": org.name,
            "slug": org.slug
        }))
        .log(&state.db)
        .await
        .ok();

    Ok((
        StatusCode::CREATED,
        Json(SignupResponse {
            user: UserInfo::from(&user),
            org: OrgInfo::from(&org),
            session_token: token,
            expires_at: session.expires_at,
        }),
    ))
}

/// Log in with email and password
pub async fn login(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<LoginRequest>,
) -> ApiResult<Json<LoginResponse>> {
    let client_info = ClientInfo::from_headers(&headers);

    // Find user by email
    let user_repo = UserRepository::new(&state.db);
    let user = user_repo
        .find_by_email(&request.email)
        .await?
        .ok_or_else(|| ApiError::Unauthorized("Invalid email or password".to_string()))?;

    // Check account status
    user.can_login().map_err(|e| match e {
        UserError::AccountSuspended => ApiError::Forbidden("Account suspended".to_string()),
        UserError::EmailNotVerified => ApiError::Forbidden("Email not verified".to_string()),
        _ => ApiError::Unauthorized("Invalid email or password".to_string()),
    })?;

    // Verify password
    if !user.verify_password(&request.password) {
        return Err(ApiError::Unauthorized(
            "Invalid email or password".to_string(),
        ));
    }

    // Update last login
    user_repo.update_last_login(user.id).await?;

    // Get user's orgs
    let user_org_repo = UserOrgRepository::new(&state.db);
    let memberships = user_org_repo.get_user_orgs(user.id).await?;

    let org_repo = OrganizationRepository::new(&state.db);
    let mut org_memberships = Vec::new();
    for membership in &memberships {
        if let Some(org) = org_repo.get_by_id(membership.org_id).await? {
            org_memberships.push(OrgMembership {
                org: OrgInfo::from(&org),
                role: membership.role,
                joined_at: membership.joined_at,
            });
        }
    }

    // Create session
    let session_repo = SessionRepository::new(&state.db);
    let (session, token) = Session::new(
        user.id,
        client_info.ip_address.clone(),
        client_info.user_agent.clone(),
        state.config.auth.jwt_expiry_hours,
    );
    session_repo.create(&session).await?;

    // Audit log: login
    if let Some(first_org) = memberships.first() {
        AuditEntry::builder(actions::USER_LOGIN, ActorType::User, user.id.to_string())
            .org_id(first_org.org_id)
            .resource(ResourceType::User, user.id.to_string())
            .ip_address(client_info.ip_address.unwrap_or_default())
            .user_agent(client_info.user_agent.unwrap_or_default())
            .log(&state.db)
            .await
            .ok();
    }

    Ok(Json(LoginResponse {
        user: UserInfo::from(&user),
        orgs: org_memberships,
        session_token: token,
        expires_at: session.expires_at,
    }))
}

/// Log out (invalidate session)
pub async fn logout(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ApiResult<StatusCode> {
    let client_info = ClientInfo::from_headers(&headers);

    // Extract session token from Authorization header
    let auth_header = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "));

    if let Some(token) = auth_header {
        if token.starts_with("rst_") {
            // It's a session token
            let session_repo = SessionRepository::new(&state.db);

            // Get session info for audit log before deleting
            if let Ok(Some(session)) = session_repo.find_by_token(token).await {
                // Get user's first org for audit log
                let user_org_repo = UserOrgRepository::new(&state.db);
                if let Ok(memberships) = user_org_repo.get_user_orgs(session.user_id).await {
                    if let Some(first_org) = memberships.first() {
                        AuditEntry::builder(
                            actions::USER_LOGOUT,
                            ActorType::User,
                            session.user_id.to_string(),
                        )
                        .org_id(first_org.org_id)
                        .resource(ResourceType::User, session.user_id.to_string())
                        .ip_address(client_info.ip_address.unwrap_or_default())
                        .user_agent(client_info.user_agent.unwrap_or_default())
                        .log(&state.db)
                        .await
                        .ok();
                    }
                }
            }

            session_repo.delete_by_token(token).await.ok();
        }
    }

    Ok(StatusCode::NO_CONTENT)
}

/// Get current user info
pub async fn get_current_user(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ApiResult<Json<LoginResponse>> {
    // Get user from session token
    let user = get_user_from_session(&state, &headers).await?;

    // Get user's orgs
    let user_org_repo = UserOrgRepository::new(&state.db);
    let memberships = user_org_repo.get_user_orgs(user.id).await?;

    let org_repo = OrganizationRepository::new(&state.db);
    let mut org_memberships = Vec::new();
    for membership in &memberships {
        if let Some(org) = org_repo.get_by_id(membership.org_id).await? {
            org_memberships.push(OrgMembership {
                org: OrgInfo::from(&org),
                role: membership.role,
                joined_at: membership.joined_at,
            });
        }
    }

    // Get current session expiry
    let token = get_session_token(&headers)?;
    let session_repo = SessionRepository::new(&state.db);
    let session = session_repo
        .find_by_token(&token)
        .await?
        .ok_or_else(|| ApiError::Unauthorized("Invalid session".to_string()))?;

    Ok(Json(LoginResponse {
        user: UserInfo::from(&user),
        orgs: org_memberships,
        session_token: token,
        expires_at: session.expires_at,
    }))
}

/// Change password (requires current password)
pub async fn change_password(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<ChangePasswordRequest>,
) -> ApiResult<StatusCode> {
    let client_info = ClientInfo::from_headers(&headers);
    let user = get_user_from_session(&state, &headers).await?;

    // Verify current password
    if !user.verify_password(&request.current_password) {
        return Err(ApiError::Unauthorized(
            "Invalid current password".to_string(),
        ));
    }

    // Validate new password
    if let Err(msg) = validate_password(&request.new_password) {
        return Err(ApiError::BadRequest(msg));
    }

    // Hash new password and update
    let new_hash = hash_password(&request.new_password)
        .map_err(|e| ApiError::Internal(format!("Password hashing failed: {}", e)))?;

    let user_repo = UserRepository::new(&state.db);
    user_repo.update_password(user.id, &new_hash).await?;

    // Invalidate all other sessions
    let session_repo = SessionRepository::new(&state.db);
    session_repo.delete_all_for_user(user.id).await?;

    // Audit log
    let user_org_repo = UserOrgRepository::new(&state.db);
    if let Ok(memberships) = user_org_repo.get_user_orgs(user.id).await {
        if let Some(first_org) = memberships.first() {
            AuditEntry::builder(
                actions::USER_PASSWORD_RESET,
                ActorType::User,
                user.id.to_string(),
            )
            .org_id(first_org.org_id)
            .resource(ResourceType::User, user.id.to_string())
            .ip_address(client_info.ip_address.unwrap_or_default())
            .user_agent(client_info.user_agent.unwrap_or_default())
            .log(&state.db)
            .await
            .ok();
        }
    }

    Ok(StatusCode::NO_CONTENT)
}

/// Request a password reset (sends email - not implemented yet)
pub async fn request_password_reset(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<RequestPasswordResetRequest>,
) -> ApiResult<StatusCode> {
    let client_info = ClientInfo::from_headers(&headers);

    // Find user by email
    let user_repo = UserRepository::new(&state.db);
    let user = match user_repo.find_by_email(&request.email).await? {
        Some(u) => u,
        None => {
            // Don't reveal if email exists
            return Ok(StatusCode::ACCEPTED);
        }
    };

    // Invalidate existing reset tokens
    let reset_repo = PasswordResetRepository::new(&state.db);
    reset_repo.invalidate_for_user(user.id).await?;

    // Create new reset token (expires in 1 hour)
    let (reset_token, raw_token) = PasswordResetToken::new(user.id, 1);
    reset_repo.create(&reset_token).await?;

    // TODO: Send email with reset link
    // For now, just log the token (in production, this would be sent via email)
    tracing::info!(
        user_id = %user.id,
        email = %user.email,
        "Password reset requested. Token: {}", raw_token
    );

    // Audit log
    let user_org_repo = UserOrgRepository::new(&state.db);
    if let Ok(memberships) = user_org_repo.get_user_orgs(user.id).await {
        if let Some(first_org) = memberships.first() {
            AuditEntry::builder(
                actions::USER_PASSWORD_RESET_REQUEST,
                ActorType::User,
                user.id.to_string(),
            )
            .org_id(first_org.org_id)
            .resource(ResourceType::User, user.id.to_string())
            .ip_address(client_info.ip_address.unwrap_or_default())
            .user_agent(client_info.user_agent.unwrap_or_default())
            .log(&state.db)
            .await
            .ok();
        }
    }

    Ok(StatusCode::ACCEPTED)
}

/// Reset password with token
pub async fn reset_password(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<ResetPasswordRequest>,
) -> ApiResult<StatusCode> {
    let client_info = ClientInfo::from_headers(&headers);

    // Validate new password
    if let Err(msg) = validate_password(&request.new_password) {
        return Err(ApiError::BadRequest(msg));
    }

    // Find reset token
    let reset_repo = PasswordResetRepository::new(&state.db);
    let reset_token = reset_repo
        .find_by_token(&request.token)
        .await?
        .ok_or_else(|| ApiError::BadRequest("Invalid or expired reset token".to_string()))?;

    if !reset_token.is_valid() {
        return Err(ApiError::BadRequest(
            "Invalid or expired reset token".to_string(),
        ));
    }

    // Hash new password and update
    let new_hash = hash_password(&request.new_password)
        .map_err(|e| ApiError::Internal(format!("Password hashing failed: {}", e)))?;

    let user_repo = UserRepository::new(&state.db);
    user_repo
        .update_password(reset_token.user_id, &new_hash)
        .await?;

    // Mark token as used
    reset_repo.mark_used(reset_token.id).await?;

    // Invalidate all sessions
    let session_repo = SessionRepository::new(&state.db);
    session_repo
        .delete_all_for_user(reset_token.user_id)
        .await?;

    // Audit log
    let user_org_repo = UserOrgRepository::new(&state.db);
    if let Ok(memberships) = user_org_repo.get_user_orgs(reset_token.user_id).await {
        if let Some(first_org) = memberships.first() {
            AuditEntry::builder(
                actions::USER_PASSWORD_RESET,
                ActorType::System,
                reset_token.user_id.to_string(),
            )
            .org_id(first_org.org_id)
            .resource(ResourceType::User, reset_token.user_id.to_string())
            .ip_address(client_info.ip_address.unwrap_or_default())
            .user_agent(client_info.user_agent.unwrap_or_default())
            .log(&state.db)
            .await
            .ok();
        }
    }

    Ok(StatusCode::NO_CONTENT)
}
