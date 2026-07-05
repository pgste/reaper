//! User authentication and management
//!
//! Provides user accounts, sessions, and org membership for the SaaS control plane.

pub mod password;
pub mod types;

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::db::Database;

pub use password::{
    generate_reset_token, generate_session_token, generate_verification_token, hash_password,
    hash_token, verify_dummy_password, verify_password,
};
pub use types::{
    EmailVerificationToken, OrgRole, PasswordResetToken, Session, User, UserError, UserOrg,
    UserStatus,
};

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
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;

        sqlx::query(
            r#"
            INSERT INTO users (id, email, email_verified, password_hash, status, created_at, updated_at, last_login_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
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
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;

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
            FROM users WHERE id = $1
            "#,
        )
        .bind(id.to_string())
        .fetch_optional(pool)
        .await?;

        row.map(|r| Self::row_to_user(r)).transpose()
    }

    /// Find user by email
    pub async fn find_by_email(&self, email: &str) -> Result<Option<User>, UserError> {
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;

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
            FROM users WHERE email = $1
            "#,
        )
        .bind(email)
        .fetch_optional(pool)
        .await?;

        row.map(|r| Self::row_to_user(r)).transpose()
    }

    /// Update user's last login time
    pub async fn update_last_login(&self, user_id: Uuid) -> Result<(), UserError> {
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;

        sqlx::query("UPDATE users SET last_login_at = $1, updated_at = $2 WHERE id = $3")
            .bind(Utc::now().to_rfc3339())
            .bind(Utc::now().to_rfc3339())
            .bind(user_id.to_string())
            .execute(pool)
            .await?;

        Ok(())
    }

    /// Update user's password
    pub async fn update_password(&self, user_id: Uuid, new_hash: &str) -> Result<(), UserError> {
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;

        sqlx::query("UPDATE users SET password_hash = $1, updated_at = $2 WHERE id = $3")
            .bind(new_hash)
            .bind(Utc::now().to_rfc3339())
            .bind(user_id.to_string())
            .execute(pool)
            .await?;

        Ok(())
    }

    /// Update user status
    pub async fn update_status(&self, user_id: Uuid, status: UserStatus) -> Result<(), UserError> {
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;

        sqlx::query("UPDATE users SET status = $1, updated_at = $2 WHERE id = $3")
            .bind(status.to_string())
            .bind(Utc::now().to_rfc3339())
            .bind(user_id.to_string())
            .execute(pool)
            .await?;

        Ok(())
    }

    /// Mark user's email as verified
    pub async fn verify_email(&self, user_id: Uuid) -> Result<(), UserError> {
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;

        sqlx::query(
            "UPDATE users SET email_verified = 1, status = 'active', updated_at = $1 WHERE id = $2",
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
            id: Uuid::parse_str(&row.0)
                .map_err(|e| UserError::Database(sqlx::Error::Decode(e.into())))?,
            email: row.1,
            email_verified: row.2 != 0,
            password_hash: row.3,
            status: row
                .4
                .parse()
                .map_err(|e: String| UserError::Database(sqlx::Error::Decode(e.into())))?,
            created_at: DateTime::parse_from_rfc3339(&row.5)
                .map(|dt| dt.with_timezone(&Utc))
                .map_err(|e| UserError::Database(sqlx::Error::Decode(e.into())))?,
            updated_at: DateTime::parse_from_rfc3339(&row.6)
                .map(|dt| dt.with_timezone(&Utc))
                .map_err(|e| UserError::Database(sqlx::Error::Decode(e.into())))?,
            last_login_at: row.7.and_then(|s| {
                DateTime::parse_from_rfc3339(&s)
                    .map(|dt| dt.with_timezone(&Utc))
                    .ok()
            }),
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
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;

        sqlx::query(
            r#"
            INSERT INTO user_orgs (id, user_id, org_id, role, invited_by, joined_at)
            VALUES ($1, $2, $3, $4, $5, $6)
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
    pub async fn get_role(
        &self,
        user_id: Uuid,
        org_id: Uuid,
    ) -> Result<Option<OrgRole>, UserError> {
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;

        let row: Option<(String,)> =
            sqlx::query_as("SELECT role FROM user_orgs WHERE user_id = $1 AND org_id = $2")
                .bind(user_id.to_string())
                .bind(org_id.to_string())
                .fetch_optional(pool)
                .await?;

        match row {
            Some((role,)) => {
                Ok(Some(role.parse().map_err(|e: String| {
                    UserError::Database(sqlx::Error::Decode(e.into()))
                })?))
            }
            None => Ok(None),
        }
    }

    /// Get all orgs for a user
    pub async fn get_user_orgs(&self, user_id: Uuid) -> Result<Vec<UserOrg>, UserError> {
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;

        let rows: Vec<(String, String, String, String, Option<String>, String)> = sqlx::query_as(
            "SELECT id, user_id, org_id, role, invited_by, joined_at FROM user_orgs WHERE user_id = $1",
        )
        .bind(user_id.to_string())
        .fetch_all(pool)
        .await?;

        rows.into_iter().map(|r| Self::row_to_user_org(r)).collect()
    }

    /// Get all members of an org
    pub async fn get_org_members(&self, org_id: Uuid) -> Result<Vec<UserOrg>, UserError> {
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;

        let rows: Vec<(String, String, String, String, Option<String>, String)> = sqlx::query_as(
            "SELECT id, user_id, org_id, role, invited_by, joined_at FROM user_orgs WHERE org_id = $1",
        )
        .bind(org_id.to_string())
        .fetch_all(pool)
        .await?;

        rows.into_iter().map(|r| Self::row_to_user_org(r)).collect()
    }

    /// Update user's role in an org
    pub async fn update_role(
        &self,
        user_id: Uuid,
        org_id: Uuid,
        new_role: OrgRole,
    ) -> Result<(), UserError> {
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;

        let result =
            sqlx::query("UPDATE user_orgs SET role = $1 WHERE user_id = $2 AND org_id = $3")
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
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;

        sqlx::query("DELETE FROM user_orgs WHERE user_id = $1 AND org_id = $2")
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
            id: Uuid::parse_str(&row.0)
                .map_err(|e| UserError::Database(sqlx::Error::Decode(e.into())))?,
            user_id: Uuid::parse_str(&row.1)
                .map_err(|e| UserError::Database(sqlx::Error::Decode(e.into())))?,
            org_id: Uuid::parse_str(&row.2)
                .map_err(|e| UserError::Database(sqlx::Error::Decode(e.into())))?,
            role: row
                .3
                .parse()
                .map_err(|e: String| UserError::Database(sqlx::Error::Decode(e.into())))?,
            invited_by: row.4.and_then(|s| Uuid::parse_str(&s).ok()),
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
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;

        sqlx::query(
            r#"
            INSERT INTO sessions (id, user_id, token_hash, ip_address, user_agent, expires_at, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
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
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;

        let token_hash = hash_token(token);

        let row: Option<(
            String,
            String,
            String,
            Option<String>,
            Option<String>,
            String,
            String,
        )> = sqlx::query_as(
            r#"
                SELECT id, user_id, token_hash, ip_address, user_agent, expires_at, created_at
                FROM sessions WHERE token_hash = $1
                "#,
        )
        .bind(&token_hash)
        .fetch_optional(pool)
        .await?;

        match row {
            Some(r) => {
                let session = Session {
                    id: Uuid::parse_str(&r.0)
                        .map_err(|e| UserError::Database(sqlx::Error::Decode(e.into())))?,
                    user_id: Uuid::parse_str(&r.1)
                        .map_err(|e| UserError::Database(sqlx::Error::Decode(e.into())))?,
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
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;

        sqlx::query("DELETE FROM sessions WHERE id = $1")
            .bind(session_id.to_string())
            .execute(pool)
            .await?;

        Ok(())
    }

    /// Delete session by token
    pub async fn delete_by_token(&self, token: &str) -> Result<(), UserError> {
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;

        let token_hash = hash_token(token);

        sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
            .bind(token_hash)
            .execute(pool)
            .await?;

        Ok(())
    }

    /// Delete all sessions for a user
    pub async fn delete_all_for_user(&self, user_id: Uuid) -> Result<(), UserError> {
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;

        sqlx::query("DELETE FROM sessions WHERE user_id = $1")
            .bind(user_id.to_string())
            .execute(pool)
            .await?;

        Ok(())
    }

    /// Clean up expired sessions
    pub async fn cleanup_expired(&self) -> Result<u64, UserError> {
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;

        let result = sqlx::query("DELETE FROM sessions WHERE expires_at < $1")
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
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;

        sqlx::query(
            r#"
            INSERT INTO password_reset_tokens (id, user_id, token_hash, expires_at, used_at, created_at)
            VALUES ($1, $2, $3, $4, $5, $6)
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
    pub async fn find_by_token(
        &self,
        token: &str,
    ) -> Result<Option<PasswordResetToken>, UserError> {
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;

        let token_hash = hash_token(token);

        let row: Option<(String, String, String, String, Option<String>, String)> = sqlx::query_as(
            "SELECT id, user_id, token_hash, expires_at, used_at, created_at FROM password_reset_tokens WHERE token_hash = $1",
        )
        .bind(&token_hash)
        .fetch_optional(pool)
        .await?;

        match row {
            Some(r) => {
                let reset_token = PasswordResetToken {
                    id: Uuid::parse_str(&r.0)
                        .map_err(|e| UserError::Database(sqlx::Error::Decode(e.into())))?,
                    user_id: Uuid::parse_str(&r.1)
                        .map_err(|e| UserError::Database(sqlx::Error::Decode(e.into())))?,
                    token_hash: r.2,
                    expires_at: DateTime::parse_from_rfc3339(&r.3)
                        .map(|dt| dt.with_timezone(&Utc))
                        .map_err(|e| UserError::Database(sqlx::Error::Decode(e.into())))?,
                    used_at: r.4.and_then(|s| {
                        DateTime::parse_from_rfc3339(&s)
                            .map(|dt| dt.with_timezone(&Utc))
                            .ok()
                    }),
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
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;

        sqlx::query("UPDATE password_reset_tokens SET used_at = $1 WHERE id = $2")
            .bind(Utc::now().to_rfc3339())
            .bind(token_id.to_string())
            .execute(pool)
            .await?;

        Ok(())
    }

    /// Invalidate all reset tokens for a user
    pub async fn invalidate_for_user(&self, user_id: Uuid) -> Result<(), UserError> {
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;

        sqlx::query("DELETE FROM password_reset_tokens WHERE user_id = $1 AND used_at IS NULL")
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
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;

        sqlx::query(
            r#"
            INSERT INTO email_verification_tokens (id, user_id, token_hash, expires_at, created_at)
            VALUES ($1, $2, $3, $4, $5)
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
    pub async fn find_by_token(
        &self,
        token: &str,
    ) -> Result<Option<EmailVerificationToken>, UserError> {
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;

        let token_hash = hash_token(token);

        let row: Option<(String, String, String, String, String)> = sqlx::query_as(
            "SELECT id, user_id, token_hash, expires_at, created_at FROM email_verification_tokens WHERE token_hash = $1",
        )
        .bind(&token_hash)
        .fetch_optional(pool)
        .await?;

        match row {
            Some(r) => {
                let verification_token = EmailVerificationToken {
                    id: Uuid::parse_str(&r.0)
                        .map_err(|e| UserError::Database(sqlx::Error::Decode(e.into())))?,
                    user_id: Uuid::parse_str(&r.1)
                        .map_err(|e| UserError::Database(sqlx::Error::Decode(e.into())))?,
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
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;

        sqlx::query("DELETE FROM email_verification_tokens WHERE id = $1")
            .bind(token_id.to_string())
            .execute(pool)
            .await?;

        Ok(())
    }

    /// Delete all verification tokens for a user
    pub async fn delete_for_user(&self, user_id: Uuid) -> Result<(), UserError> {
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;

        sqlx::query("DELETE FROM email_verification_tokens WHERE user_id = $1")
            .bind(user_id.to_string())
            .execute(pool)
            .await?;

        Ok(())
    }
}
