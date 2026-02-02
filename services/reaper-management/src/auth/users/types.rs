//! User types and domain models

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use super::password::{
    generate_reset_token, generate_session_token, generate_verification_token, hash_password,
    hash_token, verify_password,
};

/// User authentication errors
#[derive(Debug, Error)]
pub enum UserError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("User not found")]
    NotFound,
    #[error("Email already exists")]
    EmailExists,
    #[error("Invalid credentials")]
    InvalidCredentials,
    #[error("Password hashing error: {0}")]
    PasswordHash(String),
    #[error("Session expired")]
    SessionExpired,
    #[error("Session not found")]
    SessionNotFound,
    #[error("Account suspended")]
    AccountSuspended,
    #[error("Email not verified")]
    EmailNotVerified,
    #[error("Invalid token")]
    InvalidToken,
    #[error("Token expired")]
    TokenExpired,
}

/// User status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UserStatus {
    Pending,
    Active,
    Suspended,
}

impl std::fmt::Display for UserStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UserStatus::Pending => write!(f, "pending"),
            UserStatus::Active => write!(f, "active"),
            UserStatus::Suspended => write!(f, "suspended"),
        }
    }
}

impl std::str::FromStr for UserStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "pending" => Ok(UserStatus::Pending),
            "active" => Ok(UserStatus::Active),
            "suspended" => Ok(UserStatus::Suspended),
            _ => Err(format!("Invalid user status: {}", s)),
        }
    }
}

/// User role within an organization
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OrgRole {
    Owner,
    Admin,
    Developer,
    Viewer,
}

impl std::fmt::Display for OrgRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OrgRole::Owner => write!(f, "owner"),
            OrgRole::Admin => write!(f, "admin"),
            OrgRole::Developer => write!(f, "developer"),
            OrgRole::Viewer => write!(f, "viewer"),
        }
    }
}

impl std::str::FromStr for OrgRole {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "owner" => Ok(OrgRole::Owner),
            "admin" => Ok(OrgRole::Admin),
            "developer" => Ok(OrgRole::Developer),
            "viewer" => Ok(OrgRole::Viewer),
            _ => Err(format!("Invalid role: {}", s)),
        }
    }
}

impl OrgRole {
    /// Check if this role can manage other users
    pub fn can_manage_users(&self) -> bool {
        matches!(self, OrgRole::Owner | OrgRole::Admin)
    }

    /// Check if this role can manage policies
    pub fn can_manage_policies(&self) -> bool {
        matches!(self, OrgRole::Owner | OrgRole::Admin | OrgRole::Developer)
    }

    /// Check if this role can manage agents
    pub fn can_manage_agents(&self) -> bool {
        matches!(self, OrgRole::Owner | OrgRole::Admin | OrgRole::Developer)
    }

    /// Check if this role can view resources
    pub fn can_view(&self) -> bool {
        true // All roles can view
    }

    /// Check if this role can delete the org
    pub fn can_delete_org(&self) -> bool {
        matches!(self, OrgRole::Owner)
    }
}

/// User account
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub email_verified: bool,
    #[serde(skip_serializing)]
    pub password_hash: String,
    pub status: UserStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_login_at: Option<DateTime<Utc>>,
}

impl User {
    /// Create a new user with hashed password
    pub fn new(email: String, password: &str) -> Result<Self, UserError> {
        let password_hash = hash_password(password).map_err(UserError::PasswordHash)?;
        Ok(Self {
            id: Uuid::new_v4(),
            email,
            email_verified: false,
            password_hash,
            status: UserStatus::Active, // Auto-activate for now (no email verification yet)
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_login_at: None,
        })
    }

    /// Verify password against stored hash
    pub fn verify_password(&self, password: &str) -> bool {
        verify_password(password, &self.password_hash)
    }

    /// Check if account can be used
    pub fn can_login(&self) -> Result<(), UserError> {
        match self.status {
            UserStatus::Active => Ok(()),
            UserStatus::Pending => Err(UserError::EmailNotVerified),
            UserStatus::Suspended => Err(UserError::AccountSuspended),
        }
    }
}

/// User-org membership
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserOrg {
    pub id: Uuid,
    pub user_id: Uuid,
    pub org_id: Uuid,
    pub role: OrgRole,
    pub invited_by: Option<Uuid>,
    pub joined_at: DateTime<Utc>,
}

/// User session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: Uuid,
    pub user_id: Uuid,
    #[serde(skip_serializing)]
    pub token_hash: String,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

impl Session {
    /// Create a new session with a generated token
    pub fn new(
        user_id: Uuid,
        ip_address: Option<String>,
        user_agent: Option<String>,
        duration_hours: u64,
    ) -> (Self, String) {
        let token = generate_session_token();
        let token_hash = hash_token(&token);

        let session = Self {
            id: Uuid::new_v4(),
            user_id,
            token_hash,
            ip_address,
            user_agent,
            expires_at: Utc::now() + Duration::hours(duration_hours as i64),
            created_at: Utc::now(),
        };

        (session, token)
    }

    /// Check if session is expired
    pub fn is_expired(&self) -> bool {
        Utc::now() > self.expires_at
    }
}

/// Password reset token
#[derive(Debug, Clone)]
pub struct PasswordResetToken {
    pub id: Uuid,
    pub user_id: Uuid,
    pub token_hash: String,
    pub expires_at: DateTime<Utc>,
    pub used_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl PasswordResetToken {
    /// Create a new password reset token
    pub fn new(user_id: Uuid, duration_hours: u64) -> (Self, String) {
        let token = generate_reset_token();
        let token_hash = hash_token(&token);

        let reset = Self {
            id: Uuid::new_v4(),
            user_id,
            token_hash,
            expires_at: Utc::now() + Duration::hours(duration_hours as i64),
            used_at: None,
            created_at: Utc::now(),
        };

        (reset, token)
    }

    /// Check if token is valid (not expired and not used)
    pub fn is_valid(&self) -> bool {
        Utc::now() <= self.expires_at && self.used_at.is_none()
    }
}

/// Email verification token
#[derive(Debug, Clone)]
pub struct EmailVerificationToken {
    pub id: Uuid,
    pub user_id: Uuid,
    pub token_hash: String,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

impl EmailVerificationToken {
    /// Create a new email verification token
    pub fn new(user_id: Uuid, duration_hours: u64) -> (Self, String) {
        let token = generate_verification_token();
        let token_hash = hash_token(&token);

        let verification = Self {
            id: Uuid::new_v4(),
            user_id,
            token_hash,
            expires_at: Utc::now() + Duration::hours(duration_hours as i64),
            created_at: Utc::now(),
        };

        (verification, token)
    }

    /// Check if token is valid (not expired)
    pub fn is_valid(&self) -> bool {
        Utc::now() <= self.expires_at
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_user_status_parsing() {
        assert_eq!("active".parse::<UserStatus>().unwrap(), UserStatus::Active);
        assert_eq!(
            "pending".parse::<UserStatus>().unwrap(),
            UserStatus::Pending
        );
        assert_eq!(
            "suspended".parse::<UserStatus>().unwrap(),
            UserStatus::Suspended
        );
    }

    #[test]
    fn test_org_role_permissions() {
        assert!(OrgRole::Owner.can_delete_org());
        assert!(!OrgRole::Admin.can_delete_org());
        assert!(OrgRole::Admin.can_manage_users());
        assert!(!OrgRole::Developer.can_manage_users());
        assert!(OrgRole::Developer.can_manage_policies());
        assert!(!OrgRole::Viewer.can_manage_policies());
    }
}
