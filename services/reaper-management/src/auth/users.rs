//! User authentication and management
//!
//! Provides user accounts, sessions, and org membership for the SaaS control plane.

use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use chrono::{DateTime, Duration, Utc};
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

use crate::db::Database;

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
        let password_hash = hash_password(password)?;
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

// Password hashing utilities

/// Hash a password using Argon2id
pub fn hash_password(password: &str) -> Result<String, UserError> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    argon2
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|e| UserError::PasswordHash(e.to_string()))
}

/// Verify a password against a hash
pub fn verify_password(password: &str, hash: &str) -> bool {
    let parsed_hash = match PasswordHash::new(hash) {
        Ok(h) => h,
        Err(_) => return false,
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok()
}

/// Generate a session token (rst_ prefix for "reaper session token")
pub fn generate_session_token() -> String {
    let mut rng = rand::thread_rng();
    let random_bytes: Vec<u8> = (0..32).map(|_| rng.gen()).collect();
    format!("rst_{}", hex::encode(random_bytes))
}

/// Generate a password reset token
pub fn generate_reset_token() -> String {
    let mut rng = rand::thread_rng();
    let random_bytes: Vec<u8> = (0..32).map(|_| rng.gen()).collect();
    hex::encode(random_bytes)
}

/// Generate an email verification token (shorter for email-friendly URLs)
pub fn generate_verification_token() -> String {
    let mut rng = rand::thread_rng();
    let random_bytes: Vec<u8> = (0..24).map(|_| rng.gen()).collect();
    hex::encode(random_bytes)
}

/// Hash a token for storage
pub fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

/// User repository for database operations
pub struct UserRepository<'a> {
    db: &'a Database,
}

impl<'a> UserRepository<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    /// Create a new user
    pub async fn create(&self, user: &User) -> Result<(), UserError> {
        let pool = self.db.sqlite_pool().ok_or(sqlx::Error::PoolClosed)?;

        sqlx::query(
            r#"
            INSERT INTO users (id, email, email_verified, password_hash, status, created_at, updated_at, last_login_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(user.id.to_string())
        .bind(&user.email)
        .bind(user.email_verified)
        .bind(&user.password_hash)
        .bind(user.status.to_string())
        .bind(user.created_at.to_rfc3339())
        .bind(user.updated_at.to_rfc3339())
        .bind(user.last_login_at.map(|t| t.to_rfc3339()))
        .execute(pool)
        .await
        .map_err(|e| {
            if e.to_string().contains("UNIQUE constraint failed") {
                UserError::EmailExists
            } else {
                UserError::Database(e)
            }
        })?;

        Ok(())
    }

    /// Find user by ID
    pub async fn find_by_id(&self, id: Uuid) -> Result<Option<User>, UserError> {
        let pool = self.db.sqlite_pool().ok_or(sqlx::Error::PoolClosed)?;

        let row: Option<(
            String,
            String,
            i32,
            String,
            String,
            String,
            String,
            Option<String>,
        )> = sqlx::query_as(
            r#"
            SELECT id, email, email_verified, password_hash, status, created_at, updated_at, last_login_at
            FROM users WHERE id = ?
            "#,
        )
        .bind(id.to_string())
        .fetch_optional(pool)
        .await?;

        row.map(|r| Self::row_to_user(r)).transpose()
    }

    /// Find user by email
    pub async fn find_by_email(&self, email: &str) -> Result<Option<User>, UserError> {
        let pool = self.db.sqlite_pool().ok_or(sqlx::Error::PoolClosed)?;

        let row: Option<(
            String,
            String,
            i32,
            String,
            String,
            String,
            String,
            Option<String>,
        )> = sqlx::query_as(
            r#"
            SELECT id, email, email_verified, password_hash, status, created_at, updated_at, last_login_at
            FROM users WHERE email = ?
            "#,
        )
        .bind(email)
        .fetch_optional(pool)
        .await?;

        row.map(|r| Self::row_to_user(r)).transpose()
    }

    /// Update user's last login time
    pub async fn update_last_login(&self, user_id: Uuid) -> Result<(), UserError> {
        let pool = self.db.sqlite_pool().ok_or(sqlx::Error::PoolClosed)?;

        sqlx::query("UPDATE users SET last_login_at = ?, updated_at = ? WHERE id = ?")
            .bind(Utc::now().to_rfc3339())
            .bind(Utc::now().to_rfc3339())
            .bind(user_id.to_string())
            .execute(pool)
            .await?;

        Ok(())
    }

    /// Update user's password
    pub async fn update_password(&self, user_id: Uuid, new_hash: &str) -> Result<(), UserError> {
        let pool = self.db.sqlite_pool().ok_or(sqlx::Error::PoolClosed)?;

        sqlx::query("UPDATE users SET password_hash = ?, updated_at = ? WHERE id = ?")
            .bind(new_hash)
            .bind(Utc::now().to_rfc3339())
            .bind(user_id.to_string())
            .execute(pool)
            .await?;

        Ok(())
    }

    /// Update user status
    pub async fn update_status(&self, user_id: Uuid, status: UserStatus) -> Result<(), UserError> {
        let pool = self.db.sqlite_pool().ok_or(sqlx::Error::PoolClosed)?;

        sqlx::query("UPDATE users SET status = ?, updated_at = ? WHERE id = ?")
            .bind(status.to_string())
            .bind(Utc::now().to_rfc3339())
            .bind(user_id.to_string())
            .execute(pool)
            .await?;

        Ok(())
    }

    /// Mark user's email as verified
    pub async fn verify_email(&self, user_id: Uuid) -> Result<(), UserError> {
        let pool = self.db.sqlite_pool().ok_or(sqlx::Error::PoolClosed)?;

        sqlx::query(
            "UPDATE users SET email_verified = 1, status = 'active', updated_at = ? WHERE id = ?",
        )
        .bind(Utc::now().to_rfc3339())
        .bind(user_id.to_string())
        .execute(pool)
        .await?;

        Ok(())
    }

    fn row_to_user(
        row: (
            String,
            String,
            i32,
            String,
            String,
            String,
            String,
            Option<String>,
        ),
    ) -> Result<User, UserError> {
        Ok(User {
            id: Uuid::parse_str(&row.0).map_err(|e| UserError::Database(sqlx::Error::Decode(e.into())))?,
            email: row.1,
            email_verified: row.2 != 0,
            password_hash: row.3,
            status: row.4.parse().map_err(|e: String| {
                UserError::Database(sqlx::Error::Decode(e.into()))
            })?,
            created_at: DateTime::parse_from_rfc3339(&row.5)
                .map(|dt| dt.with_timezone(&Utc))
                .map_err(|e| UserError::Database(sqlx::Error::Decode(e.into())))?,
            updated_at: DateTime::parse_from_rfc3339(&row.6)
                .map(|dt| dt.with_timezone(&Utc))
                .map_err(|e| UserError::Database(sqlx::Error::Decode(e.into())))?,
            last_login_at: row.7.map(|s| {
                DateTime::parse_from_rfc3339(&s)
                    .map(|dt| dt.with_timezone(&Utc))
                    .ok()
            }).flatten(),
        })
    }
}

/// User-org membership repository
pub struct UserOrgRepository<'a> {
    db: &'a Database,
}

impl<'a> UserOrgRepository<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    /// Add user to org with role
    pub async fn add_membership(&self, membership: &UserOrg) -> Result<(), UserError> {
        let pool = self.db.sqlite_pool().ok_or(sqlx::Error::PoolClosed)?;

        sqlx::query(
            r#"
            INSERT INTO user_orgs (id, user_id, org_id, role, invited_by, joined_at)
            VALUES (?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(membership.id.to_string())
        .bind(membership.user_id.to_string())
        .bind(membership.org_id.to_string())
        .bind(membership.role.to_string())
        .bind(membership.invited_by.map(|id| id.to_string()))
        .bind(membership.joined_at.to_rfc3339())
        .execute(pool)
        .await?;

        Ok(())
    }

    /// Get user's role in an org
    pub async fn get_role(&self, user_id: Uuid, org_id: Uuid) -> Result<Option<OrgRole>, UserError> {
        let pool = self.db.sqlite_pool().ok_or(sqlx::Error::PoolClosed)?;

        let row: Option<(String,)> =
            sqlx::query_as("SELECT role FROM user_orgs WHERE user_id = ? AND org_id = ?")
                .bind(user_id.to_string())
                .bind(org_id.to_string())
                .fetch_optional(pool)
                .await?;

        match row {
            Some((role,)) => Ok(Some(role.parse().map_err(|e: String| {
                UserError::Database(sqlx::Error::Decode(e.into()))
            })?)),
            None => Ok(None),
        }
    }

    /// Get all orgs for a user
    pub async fn get_user_orgs(&self, user_id: Uuid) -> Result<Vec<UserOrg>, UserError> {
        let pool = self.db.sqlite_pool().ok_or(sqlx::Error::PoolClosed)?;

        let rows: Vec<(String, String, String, String, Option<String>, String)> = sqlx::query_as(
            "SELECT id, user_id, org_id, role, invited_by, joined_at FROM user_orgs WHERE user_id = ?",
        )
        .bind(user_id.to_string())
        .fetch_all(pool)
        .await?;

        rows.into_iter()
            .map(|r| Self::row_to_user_org(r))
            .collect()
    }

    /// Get all members of an org
    pub async fn get_org_members(&self, org_id: Uuid) -> Result<Vec<UserOrg>, UserError> {
        let pool = self.db.sqlite_pool().ok_or(sqlx::Error::PoolClosed)?;

        let rows: Vec<(String, String, String, String, Option<String>, String)> = sqlx::query_as(
            "SELECT id, user_id, org_id, role, invited_by, joined_at FROM user_orgs WHERE org_id = ?",
        )
        .bind(org_id.to_string())
        .fetch_all(pool)
        .await?;

        rows.into_iter()
            .map(|r| Self::row_to_user_org(r))
            .collect()
    }

    /// Update user's role in an org
    pub async fn update_role(
        &self,
        user_id: Uuid,
        org_id: Uuid,
        new_role: OrgRole,
    ) -> Result<(), UserError> {
        let pool = self.db.sqlite_pool().ok_or(sqlx::Error::PoolClosed)?;

        let result =
            sqlx::query("UPDATE user_orgs SET role = ? WHERE user_id = ? AND org_id = ?")
                .bind(new_role.to_string())
                .bind(user_id.to_string())
                .bind(org_id.to_string())
                .execute(pool)
                .await?;

        if result.rows_affected() == 0 {
            return Err(UserError::NotFound);
        }

        Ok(())
    }

    /// Remove user from org
    pub async fn remove_membership(&self, user_id: Uuid, org_id: Uuid) -> Result<(), UserError> {
        let pool = self.db.sqlite_pool().ok_or(sqlx::Error::PoolClosed)?;

        sqlx::query("DELETE FROM user_orgs WHERE user_id = ? AND org_id = ?")
            .bind(user_id.to_string())
            .bind(org_id.to_string())
            .execute(pool)
            .await?;

        Ok(())
    }

    fn row_to_user_org(
        row: (String, String, String, String, Option<String>, String),
    ) -> Result<UserOrg, UserError> {
        Ok(UserOrg {
            id: Uuid::parse_str(&row.0).map_err(|e| UserError::Database(sqlx::Error::Decode(e.into())))?,
            user_id: Uuid::parse_str(&row.1).map_err(|e| UserError::Database(sqlx::Error::Decode(e.into())))?,
            org_id: Uuid::parse_str(&row.2).map_err(|e| UserError::Database(sqlx::Error::Decode(e.into())))?,
            role: row.3.parse().map_err(|e: String| {
                UserError::Database(sqlx::Error::Decode(e.into()))
            })?,
            invited_by: row.4.map(|s| Uuid::parse_str(&s).ok()).flatten(),
            joined_at: DateTime::parse_from_rfc3339(&row.5)
                .map(|dt| dt.with_timezone(&Utc))
                .map_err(|e| UserError::Database(sqlx::Error::Decode(e.into())))?,
        })
    }
}

/// Session repository
pub struct SessionRepository<'a> {
    db: &'a Database,
}

impl<'a> SessionRepository<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    /// Create a new session
    pub async fn create(&self, session: &Session) -> Result<(), UserError> {
        let pool = self.db.sqlite_pool().ok_or(sqlx::Error::PoolClosed)?;

        sqlx::query(
            r#"
            INSERT INTO sessions (id, user_id, token_hash, ip_address, user_agent, expires_at, created_at)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(session.id.to_string())
        .bind(session.user_id.to_string())
        .bind(&session.token_hash)
        .bind(&session.ip_address)
        .bind(&session.user_agent)
        .bind(session.expires_at.to_rfc3339())
        .bind(session.created_at.to_rfc3339())
        .execute(pool)
        .await?;

        Ok(())
    }

    /// Find session by token (validates and returns session with user)
    pub async fn find_by_token(&self, token: &str) -> Result<Option<Session>, UserError> {
        let pool = self.db.sqlite_pool().ok_or(sqlx::Error::PoolClosed)?;

        let token_hash = hash_token(token);

        let row: Option<(String, String, String, Option<String>, Option<String>, String, String)> =
            sqlx::query_as(
                r#"
                SELECT id, user_id, token_hash, ip_address, user_agent, expires_at, created_at
                FROM sessions WHERE token_hash = ?
                "#,
            )
            .bind(&token_hash)
            .fetch_optional(pool)
            .await?;

        match row {
            Some(r) => {
                let session = Session {
                    id: Uuid::parse_str(&r.0).map_err(|e| UserError::Database(sqlx::Error::Decode(e.into())))?,
                    user_id: Uuid::parse_str(&r.1).map_err(|e| UserError::Database(sqlx::Error::Decode(e.into())))?,
                    token_hash: r.2,
                    ip_address: r.3,
                    user_agent: r.4,
                    expires_at: DateTime::parse_from_rfc3339(&r.5)
                        .map(|dt| dt.with_timezone(&Utc))
                        .map_err(|e| UserError::Database(sqlx::Error::Decode(e.into())))?,
                    created_at: DateTime::parse_from_rfc3339(&r.6)
                        .map(|dt| dt.with_timezone(&Utc))
                        .map_err(|e| UserError::Database(sqlx::Error::Decode(e.into())))?,
                };

                if session.is_expired() {
                    return Err(UserError::SessionExpired);
                }

                Ok(Some(session))
            }
            None => Ok(None),
        }
    }

    /// Delete session (logout)
    pub async fn delete(&self, session_id: Uuid) -> Result<(), UserError> {
        let pool = self.db.sqlite_pool().ok_or(sqlx::Error::PoolClosed)?;

        sqlx::query("DELETE FROM sessions WHERE id = ?")
            .bind(session_id.to_string())
            .execute(pool)
            .await?;

        Ok(())
    }

    /// Delete session by token
    pub async fn delete_by_token(&self, token: &str) -> Result<(), UserError> {
        let pool = self.db.sqlite_pool().ok_or(sqlx::Error::PoolClosed)?;

        let token_hash = hash_token(token);

        sqlx::query("DELETE FROM sessions WHERE token_hash = ?")
            .bind(token_hash)
            .execute(pool)
            .await?;

        Ok(())
    }

    /// Delete all sessions for a user
    pub async fn delete_all_for_user(&self, user_id: Uuid) -> Result<(), UserError> {
        let pool = self.db.sqlite_pool().ok_or(sqlx::Error::PoolClosed)?;

        sqlx::query("DELETE FROM sessions WHERE user_id = ?")
            .bind(user_id.to_string())
            .execute(pool)
            .await?;

        Ok(())
    }

    /// Clean up expired sessions
    pub async fn cleanup_expired(&self) -> Result<u64, UserError> {
        let pool = self.db.sqlite_pool().ok_or(sqlx::Error::PoolClosed)?;

        let result = sqlx::query("DELETE FROM sessions WHERE expires_at < ?")
            .bind(Utc::now().to_rfc3339())
            .execute(pool)
            .await?;

        Ok(result.rows_affected())
    }
}

/// Password reset token repository
pub struct PasswordResetRepository<'a> {
    db: &'a Database,
}

impl<'a> PasswordResetRepository<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    /// Create a new password reset token
    pub async fn create(&self, token: &PasswordResetToken) -> Result<(), UserError> {
        let pool = self.db.sqlite_pool().ok_or(sqlx::Error::PoolClosed)?;

        sqlx::query(
            r#"
            INSERT INTO password_reset_tokens (id, user_id, token_hash, expires_at, used_at, created_at)
            VALUES (?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(token.id.to_string())
        .bind(token.user_id.to_string())
        .bind(&token.token_hash)
        .bind(token.expires_at.to_rfc3339())
        .bind(token.used_at.map(|t| t.to_rfc3339()))
        .bind(token.created_at.to_rfc3339())
        .execute(pool)
        .await?;

        Ok(())
    }

    /// Find token by hash
    pub async fn find_by_token(&self, token: &str) -> Result<Option<PasswordResetToken>, UserError> {
        let pool = self.db.sqlite_pool().ok_or(sqlx::Error::PoolClosed)?;

        let token_hash = hash_token(token);

        let row: Option<(String, String, String, String, Option<String>, String)> = sqlx::query_as(
            "SELECT id, user_id, token_hash, expires_at, used_at, created_at FROM password_reset_tokens WHERE token_hash = ?",
        )
        .bind(&token_hash)
        .fetch_optional(pool)
        .await?;

        match row {
            Some(r) => {
                let reset_token = PasswordResetToken {
                    id: Uuid::parse_str(&r.0).map_err(|e| UserError::Database(sqlx::Error::Decode(e.into())))?,
                    user_id: Uuid::parse_str(&r.1).map_err(|e| UserError::Database(sqlx::Error::Decode(e.into())))?,
                    token_hash: r.2,
                    expires_at: DateTime::parse_from_rfc3339(&r.3)
                        .map(|dt| dt.with_timezone(&Utc))
                        .map_err(|e| UserError::Database(sqlx::Error::Decode(e.into())))?,
                    used_at: r.4.map(|s| {
                        DateTime::parse_from_rfc3339(&s)
                            .map(|dt| dt.with_timezone(&Utc))
                            .ok()
                    }).flatten(),
                    created_at: DateTime::parse_from_rfc3339(&r.5)
                        .map(|dt| dt.with_timezone(&Utc))
                        .map_err(|e| UserError::Database(sqlx::Error::Decode(e.into())))?,
                };

                Ok(Some(reset_token))
            }
            None => Ok(None),
        }
    }

    /// Mark token as used
    pub async fn mark_used(&self, token_id: Uuid) -> Result<(), UserError> {
        let pool = self.db.sqlite_pool().ok_or(sqlx::Error::PoolClosed)?;

        sqlx::query("UPDATE password_reset_tokens SET used_at = ? WHERE id = ?")
            .bind(Utc::now().to_rfc3339())
            .bind(token_id.to_string())
            .execute(pool)
            .await?;

        Ok(())
    }

    /// Invalidate all reset tokens for a user
    pub async fn invalidate_for_user(&self, user_id: Uuid) -> Result<(), UserError> {
        let pool = self.db.sqlite_pool().ok_or(sqlx::Error::PoolClosed)?;

        sqlx::query("DELETE FROM password_reset_tokens WHERE user_id = ? AND used_at IS NULL")
            .bind(user_id.to_string())
            .execute(pool)
            .await?;

        Ok(())
    }
}

/// Email verification token repository
pub struct EmailVerificationRepository<'a> {
    db: &'a Database,
}

impl<'a> EmailVerificationRepository<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    /// Create a new email verification token
    pub async fn create(&self, token: &EmailVerificationToken) -> Result<(), UserError> {
        let pool = self.db.sqlite_pool().ok_or(sqlx::Error::PoolClosed)?;

        sqlx::query(
            r#"
            INSERT INTO email_verification_tokens (id, user_id, token_hash, expires_at, created_at)
            VALUES (?, ?, ?, ?, ?)
            "#,
        )
        .bind(token.id.to_string())
        .bind(token.user_id.to_string())
        .bind(&token.token_hash)
        .bind(token.expires_at.to_rfc3339())
        .bind(token.created_at.to_rfc3339())
        .execute(pool)
        .await?;

        Ok(())
    }

    /// Find token by hash
    pub async fn find_by_token(&self, token: &str) -> Result<Option<EmailVerificationToken>, UserError> {
        let pool = self.db.sqlite_pool().ok_or(sqlx::Error::PoolClosed)?;

        let token_hash = hash_token(token);

        let row: Option<(String, String, String, String, String)> = sqlx::query_as(
            "SELECT id, user_id, token_hash, expires_at, created_at FROM email_verification_tokens WHERE token_hash = ?",
        )
        .bind(&token_hash)
        .fetch_optional(pool)
        .await?;

        match row {
            Some(r) => {
                let verification_token = EmailVerificationToken {
                    id: Uuid::parse_str(&r.0).map_err(|e| UserError::Database(sqlx::Error::Decode(e.into())))?,
                    user_id: Uuid::parse_str(&r.1).map_err(|e| UserError::Database(sqlx::Error::Decode(e.into())))?,
                    token_hash: r.2,
                    expires_at: DateTime::parse_from_rfc3339(&r.3)
                        .map(|dt| dt.with_timezone(&Utc))
                        .map_err(|e| UserError::Database(sqlx::Error::Decode(e.into())))?,
                    created_at: DateTime::parse_from_rfc3339(&r.4)
                        .map(|dt| dt.with_timezone(&Utc))
                        .map_err(|e| UserError::Database(sqlx::Error::Decode(e.into())))?,
                };

                Ok(Some(verification_token))
            }
            None => Ok(None),
        }
    }

    /// Delete verification token after successful verification
    pub async fn delete(&self, token_id: Uuid) -> Result<(), UserError> {
        let pool = self.db.sqlite_pool().ok_or(sqlx::Error::PoolClosed)?;

        sqlx::query("DELETE FROM email_verification_tokens WHERE id = ?")
            .bind(token_id.to_string())
            .execute(pool)
            .await?;

        Ok(())
    }

    /// Delete all verification tokens for a user
    pub async fn delete_for_user(&self, user_id: Uuid) -> Result<(), UserError> {
        let pool = self.db.sqlite_pool().ok_or(sqlx::Error::PoolClosed)?;

        sqlx::query("DELETE FROM email_verification_tokens WHERE user_id = ?")
            .bind(user_id.to_string())
            .execute(pool)
            .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_password_hashing() {
        let password = "SecurePass123!";
        let hash = hash_password(password).unwrap();
        assert!(verify_password(password, &hash));
        assert!(!verify_password("wrong_password", &hash));
    }

    #[test]
    fn test_session_token_generation() {
        let token = generate_session_token();
        assert!(token.starts_with("rst_"));
        assert_eq!(token.len(), 68); // "rst_" + 64 hex chars
    }

    #[test]
    fn test_token_hashing() {
        let token = "test_token";
        let hash1 = hash_token(token);
        let hash2 = hash_token(token);
        assert_eq!(hash1, hash2);
        assert_ne!(hash_token("different_token"), hash1);
    }

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
