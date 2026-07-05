//! API key management
//!
//! Provides generation, validation, and storage of API keys.

use chrono::{DateTime, Utc};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::Row;
use uuid::Uuid;

use super::scopes::Scope;
use crate::db::{Database, DatabaseError};

/// API key prefix (for identification)
pub const API_KEY_PREFIX: &str = "rpr_";
/// Length of the random part of the key
const KEY_RANDOM_LENGTH: usize = 32;

/// API key entity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKey {
    pub id: Uuid,
    pub org_id: Uuid,
    pub name: String,
    pub key_prefix: String,
    pub scopes: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub is_revoked: bool,
    pub created_at: DateTime<Utc>,
    pub created_by: Option<String>,
}

/// Input for creating an API key
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateApiKey {
    pub name: String,
    #[serde(default = "default_scopes")]
    pub scopes: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_by: Option<String>,
}

fn default_scopes() -> Vec<String> {
    Scope::agent_defaults()
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// Result of creating an API key (includes the full key, shown only once)
#[derive(Debug, Serialize)]
pub struct ApiKeyCreated {
    pub id: Uuid,
    pub name: String,
    /// The full API key - only shown once at creation
    pub key: String,
    pub key_prefix: String,
    pub scopes: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// API key generator
pub struct ApiKeyGenerator;

impl ApiKeyGenerator {
    /// Generate a new API key
    pub fn generate() -> (String, String, String) {
        let mut rng = rand::thread_rng();
        let mut random_bytes = vec![0u8; KEY_RANDOM_LENGTH];
        rng.fill_bytes(&mut random_bytes);
        let random_part = hex::encode(&random_bytes);
        let full_key = format!("{}{}", API_KEY_PREFIX, random_part);
        let prefix = format!("{}{}", API_KEY_PREFIX, &random_part[..8]);
        let hash = Self::hash_key(&full_key);
        (full_key, prefix, hash)
    }

    /// Hash an API key for storage
    pub fn hash_key(key: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(key.as_bytes());
        hex::encode(hasher.finalize())
    }

    /// Validate key format
    pub fn validate_format(key: &str) -> bool {
        key.starts_with(API_KEY_PREFIX) && key.len() == API_KEY_PREFIX.len() + KEY_RANDOM_LENGTH * 2
    }
}

/// API key repository
pub struct ApiKeyRepository<'a> {
    db: &'a Database,
}

impl<'a> ApiKeyRepository<'a> {
    /// Create a new API key repository
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    /// Create a new API key
    pub async fn create(
        &self,
        org_id: Uuid,
        input: CreateApiKey,
    ) -> Result<ApiKeyCreated, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let (full_key, prefix, hash) = ApiKeyGenerator::generate();
        let id = Uuid::new_v4();
        let now = Utc::now();
        let scopes_json = serde_json::to_string(&input.scopes).unwrap_or_else(|_| "[]".to_string());

        sqlx::query(
            r#"
            INSERT INTO api_keys (id, org_id, name, key_prefix, key_hash, scopes, expires_at, created_at, created_by)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            "#,
        )
        .bind(id.to_string())
        .bind(org_id.to_string())
        .bind(&input.name)
        .bind(&prefix)
        .bind(&hash)
        .bind(&scopes_json)
        .bind(input.expires_at.map(|d| d.to_rfc3339()))
        .bind(now.to_rfc3339())
        .bind(&input.created_by)
        .execute(pool)
        .await?;

        Ok(ApiKeyCreated {
            id,
            name: input.name,
            key: full_key,
            key_prefix: prefix,
            scopes: input.scopes,
            expires_at: input.expires_at,
            created_at: now,
        })
    }

    /// Validate an API key and return its details
    pub async fn validate(&self, key: &str) -> Result<Option<ApiKey>, DatabaseError> {
        if !ApiKeyGenerator::validate_format(key) {
            return Ok(None);
        }

        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let hash = ApiKeyGenerator::hash_key(key);

        let row = sqlx::query(
            r#"
            SELECT id, org_id, name, key_prefix, scopes, expires_at, last_used_at, is_revoked, created_at, created_by
            FROM api_keys
            WHERE key_hash = $1 AND is_revoked = 0
            "#,
        )
        .bind(&hash)
        .fetch_optional(pool)
        .await?;

        match row {
            Some(row) => {
                let api_key = self.row_to_api_key(row)?;

                // Check expiration
                if let Some(expires_at) = api_key.expires_at {
                    if expires_at < Utc::now() {
                        return Ok(None);
                    }
                }

                // Touch last_used_at OFF the request path, and only when
                // it's actually stale: the previous synchronous UPDATE put
                // a write commit inside EVERY authenticated request —
                // measured at ~75% of control-plane save latency. Audit
                // freshness of "last used" is minutes-granularity; one
                // write per key per minute preserves it.
                let needs_touch = api_key
                    .last_used_at
                    .map_or(true, |t| Utc::now() - t > chrono::Duration::seconds(60));
                if needs_touch {
                    let db = self.db.clone();
                    let key_id = api_key.id;
                    tokio::spawn(async move {
                        let repo = ApiKeyRepository::new(&db);
                        let _ = repo.update_last_used(key_id).await;
                    });
                }

                Ok(Some(api_key))
            }
            None => Ok(None),
        }
    }

    /// Get API key by ID
    pub async fn get_by_id(&self, id: Uuid) -> Result<Option<ApiKey>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let row = sqlx::query(
            r#"
            SELECT id, org_id, name, key_prefix, scopes, expires_at, last_used_at, is_revoked, created_at, created_by
            FROM api_keys
            WHERE id = $1
            "#,
        )
        .bind(id.to_string())
        .fetch_optional(pool)
        .await?;

        match row {
            Some(row) => Ok(Some(self.row_to_api_key(row)?)),
            None => Ok(None),
        }
    }

    /// List API keys for an organization
    pub async fn list_by_org(&self, org_id: Uuid) -> Result<Vec<ApiKey>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let rows = sqlx::query(
            r#"
            SELECT id, org_id, name, key_prefix, scopes, expires_at, last_used_at, is_revoked, created_at, created_by
            FROM api_keys
            WHERE org_id = $1
            ORDER BY created_at DESC
            "#,
        )
        .bind(org_id.to_string())
        .fetch_all(pool)
        .await?;

        let mut keys = Vec::with_capacity(rows.len());
        for row in rows {
            keys.push(self.row_to_api_key(row)?);
        }

        Ok(keys)
    }

    /// Revoke an API key
    pub async fn revoke(&self, id: Uuid) -> Result<bool, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let result = sqlx::query("UPDATE api_keys SET is_revoked = 1 WHERE id = $1")
            .bind(id.to_string())
            .execute(pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Delete an API key
    pub async fn delete(&self, id: Uuid) -> Result<bool, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let result = sqlx::query("DELETE FROM api_keys WHERE id = $1")
            .bind(id.to_string())
            .execute(pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Update last used timestamp
    async fn update_last_used(&self, id: Uuid) -> Result<(), DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let now = Utc::now().to_rfc3339();
        sqlx::query("UPDATE api_keys SET last_used_at = $1 WHERE id = $2")
            .bind(&now)
            .bind(id.to_string())
            .execute(pool)
            .await?;

        Ok(())
    }

    /// Convert database row to ApiKey
    fn row_to_api_key(&self, row: sqlx::any::AnyRow) -> Result<ApiKey, DatabaseError> {
        let id_str: String = row.get("id");
        let id = Uuid::parse_str(&id_str)
            .map_err(|e| DatabaseError::Config(format!("Invalid UUID: {}", e)))?;

        let org_id_str: String = row.get("org_id");
        let org_id = Uuid::parse_str(&org_id_str)
            .map_err(|e| DatabaseError::Config(format!("Invalid org UUID: {}", e)))?;

        let scopes_str: String = row.get("scopes");
        let scopes: Vec<String> = serde_json::from_str(&scopes_str).unwrap_or_else(|_| vec![]);

        let expires_at: Option<String> = row.get("expires_at");
        let expires_at = expires_at
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
            .map(|dt| dt.with_timezone(&Utc));

        let last_used_at: Option<String> = row.get("last_used_at");
        let last_used_at = last_used_at
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
            .map(|dt| dt.with_timezone(&Utc));

        let created_at_str: String = row.get("created_at");
        let created_at = chrono::DateTime::parse_from_rfc3339(&created_at_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());

        let is_revoked: i32 = row.get("is_revoked");

        Ok(ApiKey {
            id,
            org_id,
            name: row.get("name"),
            key_prefix: row.get("key_prefix"),
            scopes,
            expires_at,
            last_used_at,
            is_revoked: is_revoked != 0,
            created_at,
            created_by: row.get("created_by"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::repositories::OrganizationRepository;
    use crate::domain::organization::CreateOrganization;
    use tempfile::TempDir;

    async fn setup_db() -> (TempDir, Database) {
        let temp_dir = TempDir::new().unwrap();

        let config = crate::db::ephemeral_test_config(temp_dir.path()).await;

        let db = Database::new(&config).await.unwrap();
        db.run_migrations().await.unwrap();
        (temp_dir, db)
    }

    async fn create_test_org(db: &Database) -> Uuid {
        let repo = OrganizationRepository::new(db);
        let input = CreateOrganization {
            name: "Test Org".to_string(),
            slug: "test-org".to_string(),
            display_name: None,
            description: None,
            settings: serde_json::json!({}),
        };
        repo.create(input).await.unwrap().id
    }

    #[test]
    fn test_key_generation() {
        let (full_key, prefix, hash) = ApiKeyGenerator::generate();

        assert!(full_key.starts_with(API_KEY_PREFIX));
        assert!(prefix.starts_with(API_KEY_PREFIX));
        assert_eq!(hash.len(), 64); // SHA-256 hex

        // Verify hash is consistent
        assert_eq!(ApiKeyGenerator::hash_key(&full_key), hash);
    }

    #[test]
    fn test_key_format_validation() {
        assert!(ApiKeyGenerator::validate_format(
            "rpr_0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        ));
        assert!(!ApiKeyGenerator::validate_format("invalid"));
        assert!(!ApiKeyGenerator::validate_format("rpr_short"));
    }

    #[tokio::test]
    async fn test_create_and_validate_key() {
        let (_temp_dir, db) = setup_db().await;
        let org_id = create_test_org(&db).await;
        let repo = ApiKeyRepository::new(&db);

        let input = CreateApiKey {
            name: "Test Key".to_string(),
            scopes: vec!["agent:register".to_string(), "agent:read".to_string()],
            expires_at: None,
            created_by: Some("admin".to_string()),
        };

        let created = repo.create(org_id, input).await.unwrap();
        assert!(created.key.starts_with(API_KEY_PREFIX));

        // Validate the key
        let validated = repo.validate(&created.key).await.unwrap().unwrap();
        assert_eq!(validated.id, created.id);
        assert_eq!(validated.name, "Test Key");
        assert_eq!(validated.scopes.len(), 2);
    }

    #[tokio::test]
    async fn test_revoke_key() {
        let (_temp_dir, db) = setup_db().await;
        let org_id = create_test_org(&db).await;
        let repo = ApiKeyRepository::new(&db);

        let input = CreateApiKey {
            name: "Revoke Test".to_string(),
            scopes: vec![],
            expires_at: None,
            created_by: None,
        };

        let created = repo.create(org_id, input).await.unwrap();

        // Revoke
        repo.revoke(created.id).await.unwrap();

        // Should not validate after revocation
        let validated = repo.validate(&created.key).await.unwrap();
        assert!(validated.is_none());
    }
}
