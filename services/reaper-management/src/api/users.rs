//! User authentication API endpoints
//!
//! Provides endpoints for user signup, login, logout, and password management.

use axum::{
    extract::{Path, State},
    http::{header::HeaderMap, StatusCode},
    response::Json,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    api::error::{ApiError, ApiResult},
    audit::{actions, ActorType, AuditEntry, ClientInfo, ResourceType},
    auth::{
        middleware::RequireAuth,
        users::{
            hash_password, EmailVerificationRepository, EmailVerificationToken, OrgRole,
            PasswordResetRepository, PasswordResetToken, Session, SessionRepository, User,
            UserError, UserOrg, UserOrgRepository, UserRepository, UserStatus,
        },
    },
    db::repositories::OrganizationRepository,
    domain::{CreateOrganization, Organization},
    state::AppState,
};

/// Build user auth routes
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        // Public endpoints (no auth required)
        .route("/auth/signup", post(signup))
        .route("/auth/login", post(login))
        .route("/auth/password/reset-request", post(request_password_reset))
        .route("/auth/password/reset", post(reset_password))
        .route("/auth/email/verify", post(verify_email))
        // Authenticated endpoints
        .route("/auth/logout", post(logout))
        .route("/auth/me", get(get_current_user))
        .route("/auth/password/change", post(change_password))
        .route("/auth/email/resend", post(resend_verification))
        // Org member management
        .route(
            "/orgs/{org}/members",
            get(list_org_members).post(invite_member),
        )
        .route(
            "/orgs/{org}/members/{user_id}",
            get(get_member).delete(remove_member),
        )
        .route("/orgs/{org}/members/{user_id}/role", post(update_member_role))
}

// ==================== Request/Response Types ====================

/// Request to sign up a new user
#[derive(Debug, Deserialize)]
pub struct SignupRequest {
    pub email: String,
    pub password: String,
    pub org_name: String,
    pub org_slug: Option<String>,
}

/// Response after successful signup
#[derive(Debug, Serialize)]
pub struct SignupResponse {
    pub user: UserInfo,
    pub org: OrgInfo,
    pub session_token: String,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

/// Request to log in
#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

/// Response after successful login
#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub user: UserInfo,
    pub orgs: Vec<OrgMembership>,
    pub session_token: String,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

/// Basic user information
#[derive(Debug, Serialize)]
pub struct UserInfo {
    pub id: Uuid,
    pub email: String,
    pub email_verified: bool,
    pub status: UserStatus,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub last_login_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl From<&User> for UserInfo {
    fn from(user: &User) -> Self {
        Self {
            id: user.id,
            email: user.email.clone(),
            email_verified: user.email_verified,
            status: user.status,
            created_at: user.created_at,
            last_login_at: user.last_login_at,
        }
    }
}

/// Basic org information
#[derive(Debug, Serialize)]
pub struct OrgInfo {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
}

impl From<&Organization> for OrgInfo {
    fn from(org: &Organization) -> Self {
        Self {
            id: org.id,
            name: org.name.clone(),
            slug: org.slug.clone(),
        }
    }
}

/// User's membership in an org
#[derive(Debug, Serialize)]
pub struct OrgMembership {
    pub org: OrgInfo,
    pub role: OrgRole,
    pub joined_at: chrono::DateTime<chrono::Utc>,
}

/// Request to change password
#[derive(Debug, Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

/// Request to request a password reset
#[derive(Debug, Deserialize)]
pub struct RequestPasswordResetRequest {
    pub email: String,
}

/// Request to reset password with token
#[derive(Debug, Deserialize)]
pub struct ResetPasswordRequest {
    pub token: String,
    pub new_password: String,
}

/// Request to verify email
#[derive(Debug, Deserialize)]
pub struct VerifyEmailRequest {
    pub token: String,
}

/// Response for verification status
#[derive(Debug, Serialize)]
pub struct VerifyEmailResponse {
    pub verified: bool,
    pub message: String,
}

/// Request to invite a member
#[derive(Debug, Deserialize)]
pub struct InviteMemberRequest {
    pub email: String,
    pub role: OrgRole,
}

/// Request to update member role
#[derive(Debug, Deserialize)]
pub struct UpdateRoleRequest {
    pub role: OrgRole,
}

/// Member info response
#[derive(Debug, Serialize)]
pub struct MemberInfo {
    pub user: UserInfo,
    pub role: OrgRole,
    pub joined_at: chrono::DateTime<chrono::Utc>,
    pub invited_by: Option<Uuid>,
}

/// List members response
#[derive(Debug, Serialize)]
pub struct ListMembersResponse {
    pub members: Vec<MemberInfo>,
}

// ==================== Handlers ====================

/// Sign up a new user and create their first organization
async fn signup(
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
async fn login(
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
async fn logout(
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
async fn get_current_user(
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
async fn change_password(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<ChangePasswordRequest>,
) -> ApiResult<StatusCode> {
    let client_info = ClientInfo::from_headers(&headers);
    let user = get_user_from_session(&state, &headers).await?;

    // Verify current password
    if !user.verify_password(&request.current_password) {
        return Err(ApiError::Unauthorized("Invalid current password".to_string()));
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
async fn request_password_reset(
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
async fn reset_password(
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
    session_repo.delete_all_for_user(reset_token.user_id).await?;

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

// ==================== Email Verification ====================

/// Verify email with token
async fn verify_email(
    State(state): State<Arc<AppState>>,
    Json(request): Json<VerifyEmailRequest>,
) -> ApiResult<Json<VerifyEmailResponse>> {
    let verification_repo = EmailVerificationRepository::new(&state.db);
    let user_repo = UserRepository::new(&state.db);

    // Find and validate token
    let token = verification_repo
        .find_by_token(&request.token)
        .await?
        .ok_or_else(|| ApiError::BadRequest("Invalid verification token".to_string()))?;

    if !token.is_valid() {
        return Err(ApiError::BadRequest("Verification token has expired".to_string()));
    }

    // Get user and verify they exist
    let user = user_repo
        .find_by_id(token.user_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("User not found".to_string()))?;

    // Check if already verified
    if user.email_verified {
        // Delete the token since it's no longer needed
        verification_repo.delete(token.id).await?;
        return Ok(Json(VerifyEmailResponse {
            verified: true,
            message: "Email already verified".to_string(),
        }));
    }

    // Mark email as verified
    user_repo.verify_email(token.user_id).await?;

    // Delete the verification token
    verification_repo.delete(token.id).await?;

    // Audit log
    AuditEntry::builder(
        actions::USER_EMAIL_VERIFY,
        ActorType::User,
        token.user_id.to_string(),
    )
    .resource(ResourceType::User, token.user_id.to_string())
    .details(serde_json::json!({
        "email": user.email
    }))
    .log(&state.db)
    .await
    .ok();

    Ok(Json(VerifyEmailResponse {
        verified: true,
        message: "Email verified successfully".to_string(),
    }))
}

/// Resend verification email
async fn resend_verification(
    State(state): State<Arc<AppState>>,
    RequireAuth(auth_user): RequireAuth,
) -> ApiResult<StatusCode> {
    let user_repo = UserRepository::new(&state.db);
    let verification_repo = EmailVerificationRepository::new(&state.db);

    // Get current user (auth_user.id is the user ID for session auth)
    let user_id = Uuid::parse_str(&auth_user.id)
        .map_err(|_| ApiError::BadRequest("Invalid user ID".to_string()))?;
    let user = user_repo
        .find_by_id(user_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("User not found".to_string()))?;

    // Check if already verified
    if user.email_verified {
        return Err(ApiError::BadRequest("Email already verified".to_string()));
    }

    // Delete any existing verification tokens for this user
    verification_repo.delete_for_user(user.id).await?;

    // Create new verification token (24 hours validity)
    let (token, _raw_token) = EmailVerificationToken::new(user.id, 24);
    verification_repo.create(&token).await?;

    // In production, you would send an email here with the token
    // For now, we just create the token and return success
    // The raw_token would be included in the verification link sent via email

    Ok(StatusCode::NO_CONTENT)
}

// ==================== Org Member Management ====================

/// List members of an organization
async fn list_org_members(
    State(state): State<Arc<AppState>>,
    RequireAuth(auth_user): RequireAuth,
    Path(org): Path<String>,
) -> ApiResult<Json<ListMembersResponse>> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = crate::api::orgs::resolve_org(&org_repo, &org).await?;

    // Verify user belongs to this org
    let user_org_repo = UserOrgRepository::new(&state.db);
    let user_id = Uuid::parse_str(&auth_user.id)
        .map_err(|_| ApiError::Internal("Invalid user ID".to_string()))?;

    let _role = user_org_repo
        .get_role(user_id, organization.id)
        .await?
        .ok_or_else(|| {
            ApiError::Forbidden("You are not a member of this organization".to_string())
        })?;

    // Get all members
    let memberships = user_org_repo.get_org_members(organization.id).await?;

    let user_repo = UserRepository::new(&state.db);
    let mut members = Vec::new();
    for membership in memberships {
        if let Some(user) = user_repo.find_by_id(membership.user_id).await? {
            members.push(MemberInfo {
                user: UserInfo::from(&user),
                role: membership.role,
                joined_at: membership.joined_at,
                invited_by: membership.invited_by,
            });
        }
    }

    Ok(Json(ListMembersResponse { members }))
}

/// Get a specific member
async fn get_member(
    State(state): State<Arc<AppState>>,
    RequireAuth(auth_user): RequireAuth,
    Path((org, member_id)): Path<(String, Uuid)>,
) -> ApiResult<Json<MemberInfo>> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = crate::api::orgs::resolve_org(&org_repo, &org).await?;

    // Verify user belongs to this org
    let user_org_repo = UserOrgRepository::new(&state.db);
    let user_id = Uuid::parse_str(&auth_user.id)
        .map_err(|_| ApiError::Internal("Invalid user ID".to_string()))?;

    user_org_repo
        .get_role(user_id, organization.id)
        .await?
        .ok_or_else(|| {
            ApiError::Forbidden("You are not a member of this organization".to_string())
        })?;

    // Get the member's role
    let role = user_org_repo
        .get_role(member_id, organization.id)
        .await?
        .ok_or_else(|| ApiError::NotFound("Member not found".to_string()))?;

    // Get the membership details
    let memberships = user_org_repo.get_org_members(organization.id).await?;
    let membership = memberships
        .iter()
        .find(|m| m.user_id == member_id)
        .ok_or_else(|| ApiError::NotFound("Member not found".to_string()))?;

    let user_repo = UserRepository::new(&state.db);
    let user = user_repo
        .find_by_id(member_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("User not found".to_string()))?;

    Ok(Json(MemberInfo {
        user: UserInfo::from(&user),
        role,
        joined_at: membership.joined_at,
        invited_by: membership.invited_by,
    }))
}

/// Invite a member to an organization
async fn invite_member(
    State(state): State<Arc<AppState>>,
    RequireAuth(auth_user): RequireAuth,
    headers: HeaderMap,
    Path(org): Path<String>,
    Json(request): Json<InviteMemberRequest>,
) -> ApiResult<(StatusCode, Json<MemberInfo>)> {
    let client_info = ClientInfo::from_headers(&headers);

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = crate::api::orgs::resolve_org(&org_repo, &org).await?;

    // Verify user belongs to this org and has permission
    let user_org_repo = UserOrgRepository::new(&state.db);
    let user_id = Uuid::parse_str(&auth_user.id)
        .map_err(|_| ApiError::Internal("Invalid user ID".to_string()))?;

    let role = user_org_repo
        .get_role(user_id, organization.id)
        .await?
        .ok_or_else(|| {
            ApiError::Forbidden("You are not a member of this organization".to_string())
        })?;

    if !role.can_manage_users() {
        return Err(ApiError::Forbidden(
            "You don't have permission to invite members".to_string(),
        ));
    }

    // Can't invite someone as owner (only one owner allowed)
    if request.role == OrgRole::Owner {
        return Err(ApiError::BadRequest(
            "Cannot invite someone as owner. Use transfer ownership instead.".to_string(),
        ));
    }

    // Find the user to invite
    let user_repo = UserRepository::new(&state.db);
    let invite_user = user_repo
        .find_by_email(&request.email)
        .await?
        .ok_or_else(|| ApiError::NotFound("User not found with that email".to_string()))?;

    // Check if already a member
    if user_org_repo
        .get_role(invite_user.id, organization.id)
        .await?
        .is_some()
    {
        return Err(ApiError::Conflict(
            "User is already a member of this organization".to_string(),
        ));
    }

    // Add membership
    let membership = UserOrg {
        id: Uuid::new_v4(),
        user_id: invite_user.id,
        org_id: organization.id,
        role: request.role,
        invited_by: Some(user_id),
        joined_at: chrono::Utc::now(),
    };
    user_org_repo.add_membership(&membership).await?;

    // Audit log
    AuditEntry::builder(
        actions::ORG_MEMBER_ADD,
        ActorType::User,
        user_id.to_string(),
    )
    .org_id(organization.id)
    .resource(ResourceType::User, invite_user.id.to_string())
    .ip_address(client_info.ip_address.unwrap_or_default())
    .user_agent(client_info.user_agent.unwrap_or_default())
    .details(serde_json::json!({
        "email": invite_user.email,
        "role": request.role.to_string()
    }))
    .log(&state.db)
    .await
    .ok();

    Ok((
        StatusCode::CREATED,
        Json(MemberInfo {
            user: UserInfo::from(&invite_user),
            role: request.role,
            joined_at: membership.joined_at,
            invited_by: Some(user_id),
        }),
    ))
}

/// Update a member's role
async fn update_member_role(
    State(state): State<Arc<AppState>>,
    RequireAuth(auth_user): RequireAuth,
    headers: HeaderMap,
    Path((org, member_id)): Path<(String, Uuid)>,
    Json(request): Json<UpdateRoleRequest>,
) -> ApiResult<StatusCode> {
    let client_info = ClientInfo::from_headers(&headers);

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = crate::api::orgs::resolve_org(&org_repo, &org).await?;

    // Verify user belongs to this org and has permission
    let user_org_repo = UserOrgRepository::new(&state.db);
    let user_id = Uuid::parse_str(&auth_user.id)
        .map_err(|_| ApiError::Internal("Invalid user ID".to_string()))?;

    let role = user_org_repo
        .get_role(user_id, organization.id)
        .await?
        .ok_or_else(|| {
            ApiError::Forbidden("You are not a member of this organization".to_string())
        })?;

    if !role.can_manage_users() {
        return Err(ApiError::Forbidden(
            "You don't have permission to update member roles".to_string(),
        ));
    }

    // Can't change to/from owner
    let current_role = user_org_repo
        .get_role(member_id, organization.id)
        .await?
        .ok_or_else(|| ApiError::NotFound("Member not found".to_string()))?;

    if current_role == OrgRole::Owner || request.role == OrgRole::Owner {
        return Err(ApiError::BadRequest(
            "Cannot change owner role. Use transfer ownership instead.".to_string(),
        ));
    }

    // Can't demote yourself
    if member_id == user_id {
        return Err(ApiError::BadRequest(
            "Cannot change your own role".to_string(),
        ));
    }

    // Update role
    user_org_repo
        .update_role(member_id, organization.id, request.role)
        .await?;

    // Audit log
    AuditEntry::builder(
        actions::ORG_MEMBER_ROLE_CHANGE,
        ActorType::User,
        user_id.to_string(),
    )
    .org_id(organization.id)
    .resource(ResourceType::User, member_id.to_string())
    .ip_address(client_info.ip_address.unwrap_or_default())
    .user_agent(client_info.user_agent.unwrap_or_default())
    .details(serde_json::json!({
        "old_role": current_role.to_string(),
        "new_role": request.role.to_string()
    }))
    .log(&state.db)
    .await
    .ok();

    Ok(StatusCode::NO_CONTENT)
}

/// Remove a member from an organization
async fn remove_member(
    State(state): State<Arc<AppState>>,
    RequireAuth(auth_user): RequireAuth,
    headers: HeaderMap,
    Path((org, member_id)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    let client_info = ClientInfo::from_headers(&headers);

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = crate::api::orgs::resolve_org(&org_repo, &org).await?;

    // Verify user belongs to this org and has permission
    let user_org_repo = UserOrgRepository::new(&state.db);
    let user_id = Uuid::parse_str(&auth_user.id)
        .map_err(|_| ApiError::Internal("Invalid user ID".to_string()))?;

    let role = user_org_repo
        .get_role(user_id, organization.id)
        .await?
        .ok_or_else(|| {
            ApiError::Forbidden("You are not a member of this organization".to_string())
        })?;

    if !role.can_manage_users() {
        return Err(ApiError::Forbidden(
            "You don't have permission to remove members".to_string(),
        ));
    }

    // Can't remove owner
    let target_role = user_org_repo
        .get_role(member_id, organization.id)
        .await?
        .ok_or_else(|| ApiError::NotFound("Member not found".to_string()))?;

    if target_role == OrgRole::Owner {
        return Err(ApiError::BadRequest(
            "Cannot remove the owner. Transfer ownership first.".to_string(),
        ));
    }

    // Can't remove yourself (use leave org instead)
    if member_id == user_id {
        return Err(ApiError::BadRequest(
            "Cannot remove yourself. Use leave organization instead.".to_string(),
        ));
    }

    // Remove membership
    user_org_repo
        .remove_membership(member_id, organization.id)
        .await?;

    // Audit log
    AuditEntry::builder(
        actions::ORG_MEMBER_REMOVE,
        ActorType::User,
        user_id.to_string(),
    )
    .org_id(organization.id)
    .resource(ResourceType::User, member_id.to_string())
    .ip_address(client_info.ip_address.unwrap_or_default())
    .user_agent(client_info.user_agent.unwrap_or_default())
    .log(&state.db)
    .await
    .ok();

    Ok(StatusCode::NO_CONTENT)
}

// ==================== Helper Functions ====================

/// Get user from session token in headers
async fn get_user_from_session(state: &AppState, headers: &HeaderMap) -> ApiResult<User> {
    let token = get_session_token(headers)?;

    let session_repo = SessionRepository::new(&state.db);
    let session = session_repo
        .find_by_token(&token)
        .await
        .map_err(|e| match e {
            UserError::SessionExpired => ApiError::Unauthorized("Session expired".to_string()),
            _ => ApiError::Internal(format!("Session lookup failed: {}", e)),
        })?
        .ok_or_else(|| ApiError::Unauthorized("Invalid session".to_string()))?;

    let user_repo = UserRepository::new(&state.db);
    user_repo
        .find_by_id(session.user_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("User not found".to_string()))
}

/// Extract session token from headers
fn get_session_token(headers: &HeaderMap) -> ApiResult<String> {
    let auth_header = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .ok_or_else(|| ApiError::Unauthorized("Missing or invalid Authorization header".to_string()))?;

    if !auth_header.starts_with("rst_") {
        return Err(ApiError::Unauthorized("Invalid session token format".to_string()));
    }

    Ok(auth_header.to_string())
}

/// Validate email format
fn is_valid_email(email: &str) -> bool {
    // Basic email validation
    let parts: Vec<&str> = email.split('@').collect();
    if parts.len() != 2 {
        return false;
    }
    let (local, domain) = (parts[0], parts[1]);

    !local.is_empty()
        && !domain.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
}

/// Validate password strength
fn validate_password(password: &str) -> Result<(), String> {
    if password.len() < 8 {
        return Err("Password must be at least 8 characters long".to_string());
    }
    if password.len() > 128 {
        return Err("Password must be at most 128 characters long".to_string());
    }
    // Additional checks can be added here (uppercase, numbers, special chars)
    Ok(())
}

/// Create a URL-friendly slug from a string
fn slugify(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<&str>>()
        .join("-")
}

/// Validate org slug format
fn is_valid_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug.len() <= 50
        && slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !slug.starts_with('-')
        && !slug.ends_with('-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_email_validation() {
        assert!(is_valid_email("test@example.com"));
        assert!(is_valid_email("user.name@domain.co.uk"));
        assert!(!is_valid_email("invalid"));
        assert!(!is_valid_email("@example.com"));
        assert!(!is_valid_email("test@"));
        assert!(!is_valid_email("test@.com"));
    }

    #[test]
    fn test_password_validation() {
        assert!(validate_password("SecurePass123!").is_ok());
        assert!(validate_password("short").is_err());
        assert!(validate_password("12345678").is_ok());
    }

    #[test]
    fn test_slugify() {
        assert_eq!(slugify("Acme Corp"), "acme-corp");
        assert_eq!(slugify("My Company!"), "my-company");
        assert_eq!(slugify("hello---world"), "hello-world");
    }

    #[test]
    fn test_slug_validation() {
        assert!(is_valid_slug("acme-corp"));
        assert!(is_valid_slug("company123"));
        assert!(!is_valid_slug("-invalid"));
        assert!(!is_valid_slug("invalid-"));
        assert!(!is_valid_slug("UPPERCASE"));
    }
}
