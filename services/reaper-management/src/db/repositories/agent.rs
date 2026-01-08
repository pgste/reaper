//! Agent repository for database operations
//!
//! Handles persistence of agent records.

use chrono::{DateTime, Utc};
use sqlx::Row;
use uuid::Uuid;

use crate::db::{Database, DatabaseError};
use crate::domain::agent::{Agent, AgentStatus, RegisterAgent};

/// Repository for agent operations
pub struct AgentRepository<'a> {
    db: &'a Database,
}

impl<'a> AgentRepository<'a> {
    /// Create a new agent repository
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    /// Create a new agent (self-registration)
    pub async fn create(&self, org_id: Uuid, input: RegisterAgent) -> Result<Agent, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let id = Uuid::new_v4();
        let now = Utc::now();
        let labels_json = serde_json::to_string(&input.labels).unwrap_or_else(|_| "{}".to_string());

        sqlx::query(
            r#"
            INSERT INTO agents (id, org_id, name, hostname, version, status, labels, last_heartbeat_at, registered_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(id.to_string())
        .bind(org_id.to_string())
        .bind(&input.name)
        .bind(&input.hostname)
        .bind(&input.version)
        .bind(AgentStatus::Active.to_string())
        .bind(&labels_json)
        .bind(now.to_rfc3339())
        .bind(now.to_rfc3339())
        .bind(now.to_rfc3339())
        .execute(pool)
        .await?;

        Ok(Agent {
            id,
            org_id,
            name: input.name,
            hostname: input.hostname,
            ip_address: None,
            version: input.version,
            status: AgentStatus::Active,
            labels: input.labels,
            last_heartbeat_at: Some(now),
            registered_at: now,
            updated_at: now,
        })
    }

    /// Get agent by ID
    pub async fn get_by_id(&self, id: Uuid) -> Result<Option<Agent>, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let row = sqlx::query(
            r#"
            SELECT id, org_id, name, hostname, ip_address, version, status, labels, last_heartbeat_at, registered_at, updated_at
            FROM agents
            WHERE id = ?
            "#,
        )
        .bind(id.to_string())
        .fetch_optional(pool)
        .await?;

        match row {
            Some(row) => Ok(Some(self.row_to_agent(row)?)),
            None => Ok(None),
        }
    }

    /// Get agent by name within an organization
    pub async fn get_by_name(
        &self,
        org_id: Uuid,
        name: &str,
    ) -> Result<Option<Agent>, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let row = sqlx::query(
            r#"
            SELECT id, org_id, name, hostname, ip_address, version, status, labels, last_heartbeat_at, registered_at, updated_at
            FROM agents
            WHERE org_id = ? AND name = ?
            "#,
        )
        .bind(org_id.to_string())
        .bind(name)
        .fetch_optional(pool)
        .await?;

        match row {
            Some(row) => Ok(Some(self.row_to_agent(row)?)),
            None => Ok(None),
        }
    }

    /// List agents for an organization
    pub async fn list_by_org(&self, org_id: Uuid) -> Result<Vec<Agent>, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let rows = sqlx::query(
            r#"
            SELECT id, org_id, name, hostname, ip_address, version, status, labels, last_heartbeat_at, registered_at, updated_at
            FROM agents
            WHERE org_id = ?
            ORDER BY name ASC
            "#,
        )
        .bind(org_id.to_string())
        .fetch_all(pool)
        .await?;

        let mut agents = Vec::with_capacity(rows.len());
        for row in rows {
            agents.push(self.row_to_agent(row)?);
        }

        Ok(agents)
    }

    /// List agents by status
    pub async fn list_by_status(
        &self,
        org_id: Uuid,
        status: AgentStatus,
    ) -> Result<Vec<Agent>, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let rows = sqlx::query(
            r#"
            SELECT id, org_id, name, hostname, ip_address, version, status, labels, last_heartbeat_at, registered_at, updated_at
            FROM agents
            WHERE org_id = ? AND status = ?
            ORDER BY name ASC
            "#,
        )
        .bind(org_id.to_string())
        .bind(status.to_string())
        .fetch_all(pool)
        .await?;

        let mut agents = Vec::with_capacity(rows.len());
        for row in rows {
            agents.push(self.row_to_agent(row)?);
        }

        Ok(agents)
    }

    /// Update heartbeat timestamp
    pub async fn update_heartbeat(&self, id: Uuid) -> Result<bool, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let now = Utc::now().to_rfc3339();

        let result = sqlx::query(
            "UPDATE agents SET last_heartbeat_at = ?, status = ?, updated_at = ? WHERE id = ?",
        )
        .bind(&now)
        .bind(AgentStatus::Active.to_string())
        .bind(&now)
        .bind(id.to_string())
        .execute(pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Mark agent as inactive
    pub async fn mark_inactive(&self, id: Uuid) -> Result<bool, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let now = Utc::now().to_rfc3339();
        let result = sqlx::query("UPDATE agents SET status = ?, updated_at = ? WHERE id = ?")
            .bind(AgentStatus::Inactive.to_string())
            .bind(&now)
            .bind(id.to_string())
            .execute(pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Mark stale agents as inactive (no heartbeat within threshold)
    pub async fn mark_stale_inactive(
        &self,
        threshold: DateTime<Utc>,
    ) -> Result<usize, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let now = Utc::now().to_rfc3339();
        let result = sqlx::query(
            "UPDATE agents SET status = ?, updated_at = ? WHERE status = ? AND last_heartbeat_at < ?",
        )
        .bind(AgentStatus::Inactive.to_string())
        .bind(&now)
        .bind(AgentStatus::Active.to_string())
        .bind(threshold.to_rfc3339())
        .execute(pool)
        .await?;

        Ok(result.rows_affected() as usize)
    }

    /// Delete agent
    pub async fn delete(&self, id: Uuid) -> Result<bool, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let result = sqlx::query("DELETE FROM agents WHERE id = ?")
            .bind(id.to_string())
            .execute(pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Convert database row to Agent
    fn row_to_agent(&self, row: sqlx::sqlite::SqliteRow) -> Result<Agent, DatabaseError> {
        let id_str: String = row.get("id");
        let id = Uuid::parse_str(&id_str)
            .map_err(|e| DatabaseError::Config(format!("Invalid UUID: {}", e)))?;

        let org_id_str: String = row.get("org_id");
        let org_id = Uuid::parse_str(&org_id_str)
            .map_err(|e| DatabaseError::Config(format!("Invalid org UUID: {}", e)))?;

        let status_str: String = row.get("status");
        let status = status_str
            .parse::<AgentStatus>()
            .unwrap_or(AgentStatus::Pending);

        let labels_str: String = row.get("labels");
        let labels = serde_json::from_str(&labels_str).unwrap_or_else(|_| serde_json::json!({}));

        let last_heartbeat_at: Option<String> = row.get("last_heartbeat_at");
        let last_heartbeat_at = last_heartbeat_at
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
            .map(|dt| dt.with_timezone(&Utc));

        let registered_at_str: String = row.get("registered_at");
        let registered_at = chrono::DateTime::parse_from_rfc3339(&registered_at_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());

        let updated_at_str: String = row.get("updated_at");
        let updated_at = chrono::DateTime::parse_from_rfc3339(&updated_at_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());

        Ok(Agent {
            id,
            org_id,
            name: row.get("name"),
            hostname: row.get("hostname"),
            ip_address: row.get("ip_address"),
            version: row.get("version"),
            status,
            labels,
            last_heartbeat_at,
            registered_at,
            updated_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DatabaseConfig;
    use crate::db::repositories::OrganizationRepository;
    use crate::domain::organization::CreateOrganization;
    use tempfile::TempDir;

    async fn setup_db() -> (TempDir, Database) {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let url = format!("sqlite:{}", db_path.display());

        let config = DatabaseConfig {
            db_type: "sqlite".to_string(),
            url,
            max_connections: 5,
        };

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

    #[tokio::test]
    async fn test_create_agent() {
        let (_temp_dir, db) = setup_db().await;
        let org_id = create_test_org(&db).await;
        let repo = AgentRepository::new(&db);

        let input = RegisterAgent {
            name: "test-agent".to_string(),
            hostname: Some("test-host.local".to_string()),
            version: Some("1.0.0".to_string()),
            labels: serde_json::json!({"env": "test"}),
        };

        let agent = repo.create(org_id, input).await.unwrap();
        assert_eq!(agent.name, "test-agent");
        assert_eq!(agent.hostname, Some("test-host.local".to_string()));
        assert_eq!(agent.status, AgentStatus::Active);
    }

    #[tokio::test]
    async fn test_get_agent_by_name() {
        let (_temp_dir, db) = setup_db().await;
        let org_id = create_test_org(&db).await;
        let repo = AgentRepository::new(&db);

        let input = RegisterAgent {
            name: "unique-agent".to_string(),
            hostname: Some("host.local".to_string()),
            version: Some("1.0.0".to_string()),
            labels: serde_json::json!({}),
        };

        repo.create(org_id, input).await.unwrap();

        let found = repo.get_by_name(org_id, "unique-agent").await.unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "unique-agent");
    }

    #[tokio::test]
    async fn test_update_heartbeat() {
        let (_temp_dir, db) = setup_db().await;
        let org_id = create_test_org(&db).await;
        let repo = AgentRepository::new(&db);

        let input = RegisterAgent {
            name: "heartbeat-agent".to_string(),
            hostname: Some("host.local".to_string()),
            version: Some("1.0.0".to_string()),
            labels: serde_json::json!({}),
        };

        let agent = repo.create(org_id, input).await.unwrap();
        let old_heartbeat = agent.last_heartbeat_at;

        // Wait a bit and update heartbeat
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        repo.update_heartbeat(agent.id).await.unwrap();

        let updated = repo.get_by_id(agent.id).await.unwrap().unwrap();
        assert!(updated.last_heartbeat_at > old_heartbeat);
    }

    #[tokio::test]
    async fn test_list_agents() {
        let (_temp_dir, db) = setup_db().await;
        let org_id = create_test_org(&db).await;
        let repo = AgentRepository::new(&db);

        for i in 0..3 {
            let input = RegisterAgent {
                name: format!("agent-{}", i),
                hostname: Some(format!("host-{}.local", i)),
                version: Some("1.0.0".to_string()),
                labels: serde_json::json!({}),
            };
            repo.create(org_id, input).await.unwrap();
        }

        let agents = repo.list_by_org(org_id).await.unwrap();
        assert_eq!(agents.len(), 3);
    }
}
