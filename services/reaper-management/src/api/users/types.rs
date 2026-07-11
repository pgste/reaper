//! Request and response types for user authentication and management.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    auth::users::{OrgRole, User, UserStatus},
    domain::Organization,
};

/// Request to sign up a new user
#[derive(Debug, Deserialize, ToSchema)]
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
#[derive(Debug, Deserialize, ToSchema)]
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
#[derive(Debug, Deserialize, ToSchema)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

/// Request to request a password reset
#[derive(Debug, Deserialize, ToSchema)]
pub struct RequestPasswordResetRequest {
    pub email: String,
}

/// Request to reset password with token
#[derive(Debug, Deserialize, ToSchema)]
pub struct ResetPasswordRequest {
    pub token: String,
    pub new_password: String,
}

/// Request to verify email
#[derive(Debug, Deserialize, ToSchema)]
pub struct VerifyEmailRequest {
    pub token: String,
}

/// Response for verification status
#[derive(Debug, Serialize, ToSchema)]
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
