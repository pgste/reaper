//! Landscape service for fleet visibility
//!
//! Aggregates agent status, bundle distribution, and metrics across the fleet.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;
use uuid::Uuid;

use crate::db::repositories::{AgentRepository, BundleRepository, NamespaceRepository};
use crate::db::Database;
use crate::domain::agent::{AgentMetrics, AgentStatus};

/// Landscape service errors
#[derive(Debug, Error)]
pub enum LandscapeError {
    #[error("Organization not found: {0}")]
    OrgNotFound(String),
    #[error("Namespace not found: {0}")]
    NamespaceNotFound(String),
    #[error("Database error: {0}")]
    Database(#[from] crate::db::DatabaseError),
}

/// Summary view of the landscape
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LandscapeSummary {
    /// Total number of agents
    pub total_agents: usize,
    /// Number of healthy/active agents
    pub healthy: usize,
    /// Number of unhealthy agents (stale heartbeat)
    pub unhealthy: usize,
    /// Number of agents pending update
    pub pending_update: usize,
    /// Number of agents with version pins
    pub pinned: usize,
}

/// Entry for a single agent in the landscape
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentEntry {
    pub id: Uuid,
    pub name: String,
    pub hostname: Option<String>,
    pub status: AgentStatus,
    pub labels: serde_json::Value,
    pub last_heartbeat_at: Option<DateTime<Utc>>,
    pub is_healthy: bool,
    pub current_bundle_id: Option<Uuid>,
    pub current_bundle_version: Option<String>,
    pub metrics: Option<AgentMetricsSummary>,
}

/// Summarized metrics for display
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMetricsSummary {
    pub requests_total: u64,
    pub requests_per_second: f64,
    pub p99_latency_us: f64,
    pub memory_mb: f64,
    pub allow_rate: f64,
    pub uptime_seconds: u64,
}

impl From<AgentMetrics> for AgentMetricsSummary {
    fn from(m: AgentMetrics) -> Self {
        let total_decisions = m.decisions_allow + m.decisions_deny;
        let allow_rate = if total_decisions > 0 {
            (m.decisions_allow as f64 / total_decisions as f64) * 100.0
        } else {
            0.0
        };

        Self {
            requests_total: m.requests_total,
            requests_per_second: m.requests_per_second,
            p99_latency_us: m.p99_latency_us,
            memory_mb: m.memory_bytes as f64 / (1024.0 * 1024.0),
            allow_rate,
            uptime_seconds: m.uptime_seconds,
        }
    }
}

/// Distribution of bundles across agents
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleDistribution {
    pub bundle_id: Uuid,
    pub bundle_name: String,
    pub version: Option<String>,
    pub agent_count: usize,
    pub percentage: f64,
    pub is_promoted: bool,
}

/// Complete landscape view
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LandscapeView {
    pub org_id: Uuid,
    pub namespace_id: Option<Uuid>,
    pub summary: LandscapeSummary,
    pub agents: Vec<AgentEntry>,
    pub bundle_distribution: Vec<BundleDistribution>,
    pub generated_at: DateTime<Utc>,
}

/// Aggregated organization metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrgMetrics {
    pub org_id: Uuid,
    pub total_agents: usize,
    pub healthy_agents: usize,
    pub total_requests: u64,
    pub avg_requests_per_second: f64,
    pub avg_latency_p99_us: f64,
    pub total_allow_decisions: u64,
    pub total_deny_decisions: u64,
    pub allow_rate_percent: f64,
    pub total_memory_mb: f64,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
}

/// Service for generating landscape views
pub struct LandscapeService {
    db: Arc<Database>,
    /// Heartbeat threshold in seconds (default 60)
    heartbeat_threshold_secs: i64,
}

impl LandscapeService {
    /// Create a new landscape service
    pub fn new(db: Arc<Database>) -> Self {
        Self {
            db,
            heartbeat_threshold_secs: 60,
        }
    }

    /// Set the heartbeat threshold for health checks
    pub fn with_heartbeat_threshold(mut self, seconds: i64) -> Self {
        self.heartbeat_threshold_secs = seconds;
        self
    }

    /// Get the full landscape view for an organization
    pub async fn get_landscape(
        &self,
        org_id: Uuid,
        namespace_id: Option<Uuid>,
    ) -> Result<LandscapeView, LandscapeError> {
        let agent_repo = AgentRepository::new(&self.db);
        let bundle_repo = BundleRepository::new(&self.db);

        // Get all agents with their metrics
        let agents_with_metrics = agent_repo.list_with_metrics(org_id).await?;

        // If namespace filter, get agents subscribed to that namespace
        let filtered_agents = if let Some(ns_id) = namespace_id {
            let ns_repo = NamespaceRepository::new(&self.db);
            let subscriptions_by_ns = self.get_agents_by_namespace(&ns_repo, ns_id).await?;

            agents_with_metrics
                .into_iter()
                .filter(|(a, _)| subscriptions_by_ns.contains(&a.id))
                .collect()
        } else {
            agents_with_metrics
        };

        // Build agent entries
        let mut agent_entries = Vec::with_capacity(filtered_agents.len());
        let mut bundle_counts: HashMap<Uuid, (String, Option<String>, usize)> = HashMap::new();

        for (agent, metrics) in &filtered_agents {
            let is_healthy = agent.is_healthy(self.heartbeat_threshold_secs);

            let (current_bundle_id, current_bundle_version) = if let Some(m) = metrics {
                (m.current_bundle_id, m.current_bundle_version.clone())
            } else {
                (None, None)
            };

            // Track bundle distribution
            if let Some(bundle_id) = current_bundle_id {
                let entry = bundle_counts
                    .entry(bundle_id)
                    .or_insert(("Unknown".to_string(), None, 0));
                entry.2 += 1;
            }

            agent_entries.push(AgentEntry {
                id: agent.id,
                name: agent.name.clone(),
                hostname: agent.hostname.clone(),
                status: agent.status,
                labels: agent.labels.clone(),
                last_heartbeat_at: agent.last_heartbeat_at,
                is_healthy,
                current_bundle_id,
                current_bundle_version,
                metrics: metrics.clone().map(Into::into),
            });
        }

        // Enrich bundle distribution with names
        let bundles = bundle_repo.list_by_org(org_id, None).await?;
        for bundle in &bundles {
            if let Some(entry) = bundle_counts.get_mut(&bundle.id) {
                entry.0 = bundle.name.clone();
                entry.1 = bundle.checksum.clone();
            }
        }

        // Get promoted bundle
        let promoted_bundle = bundle_repo.get_promoted(org_id).await?;

        // Build bundle distribution
        let total_with_bundle = bundle_counts.values().map(|(_, _, c)| c).sum::<usize>().max(1);
        let bundle_distribution: Vec<BundleDistribution> = bundle_counts
            .into_iter()
            .map(|(bundle_id, (name, version, count))| {
                let is_promoted = promoted_bundle
                    .as_ref()
                    .map(|p| p.id == bundle_id)
                    .unwrap_or(false);
                BundleDistribution {
                    bundle_id,
                    bundle_name: name,
                    version,
                    agent_count: count,
                    percentage: (count as f64 / total_with_bundle as f64) * 100.0,
                    is_promoted,
                }
            })
            .collect();

        // Calculate summary
        let total_agents = agent_entries.len();
        let healthy = agent_entries.iter().filter(|a| a.is_healthy).count();
        let unhealthy = total_agents - healthy;

        // Count pending updates (agents not on promoted bundle)
        let pending_update = if let Some(promoted) = &promoted_bundle {
            agent_entries
                .iter()
                .filter(|a| a.current_bundle_id != Some(promoted.id))
                .count()
        } else {
            0
        };

        Ok(LandscapeView {
            org_id,
            namespace_id,
            summary: LandscapeSummary {
                total_agents,
                healthy,
                unhealthy,
                pending_update,
                pinned: 0, // TODO: Count from version_pins table
            },
            agents: agent_entries,
            bundle_distribution,
            generated_at: Utc::now(),
        })
    }

    /// Get aggregated metrics for an organization
    pub async fn get_org_metrics(&self, org_id: Uuid) -> Result<OrgMetrics, LandscapeError> {
        let agent_repo = AgentRepository::new(&self.db);
        let agents_with_metrics = agent_repo.list_with_metrics(org_id).await?;

        let mut total_requests = 0u64;
        let mut total_rps = 0.0f64;
        let mut total_latency = 0.0f64;
        let mut total_allow = 0u64;
        let mut total_deny = 0u64;
        let mut total_memory = 0.0f64;
        let mut agents_with_metrics_count = 0usize;

        for (_agent, metrics) in &agents_with_metrics {
            if let Some(m) = metrics {
                total_requests += m.requests_total;
                total_rps += m.requests_per_second;
                total_latency += m.p99_latency_us;
                total_allow += m.decisions_allow;
                total_deny += m.decisions_deny;
                total_memory += m.memory_bytes as f64 / (1024.0 * 1024.0);
                agents_with_metrics_count += 1;
            }
        }

        let healthy_agents = agents_with_metrics
            .iter()
            .filter(|(a, _)| a.is_healthy(self.heartbeat_threshold_secs))
            .count();

        let avg_latency = if agents_with_metrics_count > 0 {
            total_latency / agents_with_metrics_count as f64
        } else {
            0.0
        };

        let total_decisions = total_allow + total_deny;
        let allow_rate = if total_decisions > 0 {
            (total_allow as f64 / total_decisions as f64) * 100.0
        } else {
            0.0
        };

        Ok(OrgMetrics {
            org_id,
            total_agents: agents_with_metrics.len(),
            healthy_agents,
            total_requests,
            avg_requests_per_second: total_rps,
            avg_latency_p99_us: avg_latency,
            total_allow_decisions: total_allow,
            total_deny_decisions: total_deny,
            allow_rate_percent: allow_rate,
            total_memory_mb: total_memory,
            period_start: Utc::now() - chrono::Duration::hours(1),
            period_end: Utc::now(),
        })
    }

    /// Get agent metrics
    pub async fn get_agent_metrics(
        &self,
        agent_id: Uuid,
    ) -> Result<Option<AgentMetrics>, LandscapeError> {
        let agent_repo = AgentRepository::new(&self.db);
        Ok(agent_repo.get_metrics(agent_id).await?)
    }

    /// Helper to get agents subscribed to a namespace
    async fn get_agents_by_namespace(
        &self,
        ns_repo: &NamespaceRepository<'_>,
        namespace_id: Uuid,
    ) -> Result<Vec<Uuid>, LandscapeError> {
        // Get all subscriptions for this namespace
        let subscriptions = ns_repo.list_subscriptions_for_namespace(namespace_id).await?;
        Ok(subscriptions.into_iter().map(|s| s.agent_id).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DatabaseConfig;
    use tempfile::TempDir;

    async fn setup_db() -> (TempDir, Arc<Database>) {
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
        (temp_dir, Arc::new(db))
    }

    async fn create_test_org(db: &Database) -> Uuid {
        let pool = db.sqlite_pool().unwrap();
        let org_id = Uuid::new_v4();
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO organizations (id, name, slug, created_at, updated_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(org_id.to_string())
        .bind("Test Org")
        .bind("test-org")
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();
        org_id
    }

    async fn create_test_agents(db: &Database, org_id: Uuid, count: usize) -> Vec<Uuid> {
        let pool = db.sqlite_pool().unwrap();
        let mut agent_ids = Vec::new();
        let now = chrono::Utc::now().to_rfc3339();

        for i in 0..count {
            let agent_id = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO agents (id, org_id, name, status, labels, last_heartbeat_at, registered_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(agent_id.to_string())
            .bind(org_id.to_string())
            .bind(format!("agent-{}", i))
            .bind("active")
            .bind("{}")
            .bind(&now)
            .bind(&now)
            .bind(&now)
            .execute(pool)
            .await
            .unwrap();

            agent_ids.push(agent_id);
        }

        agent_ids
    }

    #[tokio::test]
    async fn test_get_landscape() {
        let (_temp_dir, db) = setup_db().await;
        let org_id = create_test_org(&db).await;
        let _agents = create_test_agents(&db, org_id, 3).await;

        let service = LandscapeService::new(db);
        let landscape = service.get_landscape(org_id, None).await.unwrap();

        assert_eq!(landscape.summary.total_agents, 3);
        assert_eq!(landscape.agents.len(), 3);
    }

    #[tokio::test]
    async fn test_get_org_metrics() {
        let (_temp_dir, db) = setup_db().await;
        let org_id = create_test_org(&db).await;
        let _agents = create_test_agents(&db, org_id, 2).await;

        let service = LandscapeService::new(db);
        let metrics = service.get_org_metrics(org_id).await.unwrap();

        assert_eq!(metrics.total_agents, 2);
    }
}
