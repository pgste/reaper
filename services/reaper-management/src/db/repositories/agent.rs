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
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let id = Uuid::new_v4();
        let now = Utc::now();
        let labels_json = serde_json::to_string(&input.labels).unwrap_or_else(|_| "{}".to_string());

        sqlx::query(
            r#"
            INSERT INTO agents (id, org_id, name, hostname, version, status, labels, last_heartbeat_at, registered_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
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
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let row = sqlx::query(
            r#"
            SELECT id, org_id, name, hostname, ip_address, version, status, labels, last_heartbeat_at, registered_at, updated_at
            FROM agents
            WHERE id = $1
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
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let row = sqlx::query(
            r#"
            SELECT id, org_id, name, hostname, ip_address, version, status, labels, last_heartbeat_at, registered_at, updated_at
            FROM agents
            WHERE org_id = $1 AND name = $2
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
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let rows = sqlx::query(
            r#"
            SELECT id, org_id, name, hostname, ip_address, version, status, labels, last_heartbeat_at, registered_at, updated_at
            FROM agents
            WHERE org_id = $1
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

    /// Keyset-paginated listing (Plan 07 Phase E): rows strictly after the
    /// `(registered_at, id)` position in `ORDER BY registered_at DESC, id DESC`
    /// order. Unlike OFFSET, the walk never drifts under concurrent inserts
    /// and stays O(page) on deep pages. `fetch` is `page limit + 1` — the
    /// caller uses the sentinel row to detect whether another page exists.
    pub async fn list_page_by_org(
        &self,
        org_id: Uuid,
        fetch: i64,
        after: Option<&(String, String)>,
    ) -> Result<Vec<Agent>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let rows = if let Some((registered_at, id)) = after {
            sqlx::query(
                r#"
                SELECT id, org_id, name, hostname, ip_address, version, status, labels, last_heartbeat_at, registered_at, updated_at
                FROM agents
                WHERE org_id = $1 AND (registered_at, id) < ($2, $3)
                ORDER BY registered_at DESC, id DESC
                LIMIT $4
                "#,
            )
            .bind(org_id.to_string())
            .bind(registered_at)
            .bind(id)
            .bind(fetch)
            .fetch_all(pool)
            .await?
        } else {
            sqlx::query(
                r#"
                SELECT id, org_id, name, hostname, ip_address, version, status, labels, last_heartbeat_at, registered_at, updated_at
                FROM agents
                WHERE org_id = $1
                ORDER BY registered_at DESC, id DESC
                LIMIT $2
                "#,
            )
            .bind(org_id.to_string())
            .bind(fetch)
            .fetch_all(pool)
            .await?
        };

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
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let rows = sqlx::query(
            r#"
            SELECT id, org_id, name, hostname, ip_address, version, status, labels, last_heartbeat_at, registered_at, updated_at
            FROM agents
            WHERE org_id = $1 AND status = $2
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
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let now = Utc::now().to_rfc3339();

        let result = sqlx::query(
            "UPDATE agents SET last_heartbeat_at = $1, status = $2, updated_at = $3 WHERE id = $4",
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
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let now = Utc::now().to_rfc3339();
        let result = sqlx::query("UPDATE agents SET status = $1, updated_at = $2 WHERE id = $3")
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
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let now = Utc::now().to_rfc3339();
        let result = sqlx::query(
            "UPDATE agents SET status = $1, updated_at = $2 WHERE status = $3 AND last_heartbeat_at < $4",
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
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let result = sqlx::query("DELETE FROM agents WHERE id = $1")
            .bind(id.to_string())
            .execute(pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Convert database row to Agent
    fn row_to_agent(&self, row: sqlx::any::AnyRow) -> Result<Agent, DatabaseError> {
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

    /// Update agent metrics (upsert into agent_metrics_latest)
    pub async fn update_metrics(
        &self,
        agent_id: Uuid,
        metrics: &crate::domain::agent::AgentMetrics,
    ) -> Result<(), DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let now = Utc::now().to_rfc3339();

        let sql = r#"
            INSERT INTO agent_metrics_latest (
                agent_id, requests_total, requests_per_second,
                latency_p50_us, latency_p99_us, decisions_allow, decisions_deny,
                memory_bytes, current_bundle_id, current_bundle_version, updated_at,
                data_version, data_applied_seq, data_stale
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
            ON CONFLICT(agent_id) DO UPDATE SET
                requests_total = excluded.requests_total,
                requests_per_second = excluded.requests_per_second,
                latency_p50_us = excluded.latency_p50_us,
                latency_p99_us = excluded.latency_p99_us,
                decisions_allow = excluded.decisions_allow,
                decisions_deny = excluded.decisions_deny,
                memory_bytes = excluded.memory_bytes,
                current_bundle_id = excluded.current_bundle_id,
                current_bundle_version = excluded.current_bundle_version,
                updated_at = excluded.updated_at,
                data_version = excluded.data_version,
                data_applied_seq = excluded.data_applied_seq,
                data_stale = excluded.data_stale
        "#;

        sqlx::query(sql)
            .bind(agent_id.to_string())
            .bind(metrics.requests_total as i64)
            .bind(metrics.requests_per_second)
            .bind(metrics.p50_latency_us)
            .bind(metrics.p99_latency_us)
            .bind(metrics.decisions_allow as i64)
            .bind(metrics.decisions_deny as i64)
            .bind(metrics.memory_bytes as i64)
            .bind(metrics.current_bundle_id.map(|id| id.to_string()))
            .bind(&metrics.current_bundle_version)
            .bind(&now)
            .bind(metrics.data_version)
            .bind(metrics.data_applied_seq)
            .bind(metrics.data_stale.map(|b| b as i64))
            .execute(pool)
            .await?;

        Ok(())
    }

    /// Get latest metrics for an agent
    pub async fn get_metrics(
        &self,
        agent_id: Uuid,
    ) -> Result<Option<crate::domain::agent::AgentMetrics>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let sql = r#"
            SELECT requests_total, requests_per_second, latency_p50_us, latency_p99_us,
                   decisions_allow, decisions_deny, memory_bytes,
                   current_bundle_id, current_bundle_version,
                   data_version, data_applied_seq, data_stale
            FROM agent_metrics_latest
            WHERE agent_id = $1
        "#;

        let row = sqlx::query(sql)
            .bind(agent_id.to_string())
            .fetch_optional(pool)
            .await?;

        Ok(row.map(|r| {
            let current_bundle_id: Option<String> = r.get("current_bundle_id");
            crate::domain::agent::AgentMetrics {
                requests_total: r.get::<i64, _>("requests_total") as u64,
                requests_per_second: r.get("requests_per_second"),
                avg_latency_us: 0.0, // Computed from p50
                p50_latency_us: r.get("latency_p50_us"),
                p99_latency_us: r.get("latency_p99_us"),
                memory_bytes: r.get::<i64, _>("memory_bytes") as u64,
                cpu_percent: 0.0, // Not stored
                decisions_allow: r.get::<i64, _>("decisions_allow") as u64,
                decisions_deny: r.get::<i64, _>("decisions_deny") as u64,
                uptime_seconds: 0, // Not stored
                current_bundle_id: current_bundle_id.and_then(|s| Uuid::parse_str(&s).ok()),
                current_bundle_version: r.get("current_bundle_version"),
                data_version: r.get("data_version"),
                data_applied_seq: r.get("data_applied_seq"),
                data_stale: r.get::<Option<i64>, _>("data_stale").map(|v| v != 0),
            }
        }))
    }

    /// Get all agents with their latest metrics for an organization
    pub async fn list_with_metrics(
        &self,
        org_id: Uuid,
    ) -> Result<Vec<(Agent, Option<crate::domain::agent::AgentMetrics>)>, DatabaseError> {
        let agents = self.list_by_org(org_id).await?;
        let mut results = Vec::with_capacity(agents.len());

        for agent in agents {
            let metrics = self.get_metrics(agent.id).await?;
            results.push((agent, metrics));
        }

        Ok(results)
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
