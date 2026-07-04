//! Application state for Reaper Management Server
//!
//! Holds shared state accessible to all request handlers.

use crate::auth::JwksValidator;
use crate::bundle::{BundleService, BundleSigner};
use crate::config::Config;
use crate::db::Database;
use crate::graceful::ShutdownSignal;
use crate::storage::BundleStorage;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::broadcast;

/// Event types for SSE broadcasting
#[derive(Debug, Clone)]
pub enum ServerEvent {
    /// Policy was updated
    PolicyUpdated {
        policy_id: uuid::Uuid,
        org_id: uuid::Uuid,
        namespace_id: Option<uuid::Uuid>,
        version: i32,
    },
    /// Policy was deleted
    PolicyDeleted {
        policy_id: uuid::Uuid,
        org_id: uuid::Uuid,
        namespace_id: Option<uuid::Uuid>,
    },
    /// Bundle was promoted
    BundlePromoted {
        bundle_id: uuid::Uuid,
        org_id: uuid::Uuid,
        namespace_id: Option<uuid::Uuid>,
        version: String,
        download_url: String,
    },
    /// Bundle was staged
    BundleStaged {
        bundle_id: uuid::Uuid,
        org_id: uuid::Uuid,
        namespace_id: Option<uuid::Uuid>,
    },
    /// Data source refresh notification
    DataRefresh {
        source_id: uuid::Uuid,
        org_id: uuid::Uuid,
        namespace_id: Option<uuid::Uuid>,
        source_type: String,
    },
    /// Keep-alive ping
    Ping {
        timestamp: chrono::DateTime<chrono::Utc>,
    },
    /// Source sync started
    SyncStarted {
        source_id: uuid::Uuid,
        source_name: String,
        org_id: uuid::Uuid,
        namespace_id: Option<uuid::Uuid>,
    },
    /// Source sync completed successfully
    SyncCompleted {
        source_id: uuid::Uuid,
        source_name: String,
        org_id: uuid::Uuid,
        namespace_id: Option<uuid::Uuid>,
        policies_updated: u32,
        duration_ms: u64,
    },
    /// Source sync failed
    SyncFailed {
        source_id: uuid::Uuid,
        source_name: String,
        org_id: uuid::Uuid,
        namespace_id: Option<uuid::Uuid>,
        error: String,
    },
    /// Agent registered
    AgentRegistered {
        agent_id: uuid::Uuid,
        agent_name: String,
        org_id: uuid::Uuid,
        namespace_id: Option<uuid::Uuid>,
    },
    /// Agent became unhealthy (missed heartbeats)
    AgentUnhealthy {
        agent_id: uuid::Uuid,
        agent_name: String,
        org_id: uuid::Uuid,
        namespace_id: Option<uuid::Uuid>,
        last_seen: chrono::DateTime<chrono::Utc>,
    },
    /// Agent came back online
    AgentHealthy {
        agent_id: uuid::Uuid,
        agent_name: String,
        org_id: uuid::Uuid,
        namespace_id: Option<uuid::Uuid>,
    },
    /// Rollout started
    RolloutStarted {
        rollout_id: uuid::Uuid,
        bundle_id: uuid::Uuid,
        org_id: uuid::Uuid,
        namespace_id: Option<uuid::Uuid>,
    },
    /// Rollout wave completed
    RolloutWaveCompleted {
        rollout_id: uuid::Uuid,
        wave_number: u32,
        org_id: uuid::Uuid,
        namespace_id: Option<uuid::Uuid>,
    },
    /// Rollout completed
    RolloutCompleted {
        rollout_id: uuid::Uuid,
        bundle_id: uuid::Uuid,
        org_id: uuid::Uuid,
        namespace_id: Option<uuid::Uuid>,
        success: bool,
    },
}

impl ServerEvent {
    /// Get the organization ID for this event
    pub fn org_id(&self) -> Option<uuid::Uuid> {
        match self {
            ServerEvent::PolicyUpdated { org_id, .. } => Some(*org_id),
            ServerEvent::PolicyDeleted { org_id, .. } => Some(*org_id),
            ServerEvent::BundlePromoted { org_id, .. } => Some(*org_id),
            ServerEvent::BundleStaged { org_id, .. } => Some(*org_id),
            ServerEvent::DataRefresh { org_id, .. } => Some(*org_id),
            ServerEvent::SyncStarted { org_id, .. } => Some(*org_id),
            ServerEvent::SyncCompleted { org_id, .. } => Some(*org_id),
            ServerEvent::SyncFailed { org_id, .. } => Some(*org_id),
            ServerEvent::AgentRegistered { org_id, .. } => Some(*org_id),
            ServerEvent::AgentUnhealthy { org_id, .. } => Some(*org_id),
            ServerEvent::AgentHealthy { org_id, .. } => Some(*org_id),
            ServerEvent::RolloutStarted { org_id, .. } => Some(*org_id),
            ServerEvent::RolloutWaveCompleted { org_id, .. } => Some(*org_id),
            ServerEvent::RolloutCompleted { org_id, .. } => Some(*org_id),
            ServerEvent::Ping { .. } => None,
        }
    }

    /// Get the namespace ID for this event (if any)
    pub fn namespace_id(&self) -> Option<uuid::Uuid> {
        match self {
            ServerEvent::PolicyUpdated { namespace_id, .. } => *namespace_id,
            ServerEvent::PolicyDeleted { namespace_id, .. } => *namespace_id,
            ServerEvent::BundlePromoted { namespace_id, .. } => *namespace_id,
            ServerEvent::BundleStaged { namespace_id, .. } => *namespace_id,
            ServerEvent::DataRefresh { namespace_id, .. } => *namespace_id,
            ServerEvent::SyncStarted { namespace_id, .. } => *namespace_id,
            ServerEvent::SyncCompleted { namespace_id, .. } => *namespace_id,
            ServerEvent::SyncFailed { namespace_id, .. } => *namespace_id,
            ServerEvent::AgentRegistered { namespace_id, .. } => *namespace_id,
            ServerEvent::AgentUnhealthy { namespace_id, .. } => *namespace_id,
            ServerEvent::AgentHealthy { namespace_id, .. } => *namespace_id,
            ServerEvent::RolloutStarted { namespace_id, .. } => *namespace_id,
            ServerEvent::RolloutWaveCompleted { namespace_id, .. } => *namespace_id,
            ServerEvent::RolloutCompleted { namespace_id, .. } => *namespace_id,
            ServerEvent::Ping { .. } => None,
        }
    }

    /// Check if this event matches a set of subscribed namespaces
    pub fn matches_subscriptions(&self, subscriptions: &[uuid::Uuid]) -> bool {
        // Ping events always match
        if matches!(self, ServerEvent::Ping { .. }) {
            return true;
        }

        // Events without a namespace match all subscriptions (org-wide events)
        match self.namespace_id() {
            None => true,
            Some(ns_id) => subscriptions.contains(&ns_id),
        }
    }
}

/// Shared application state
#[derive(Clone)]
pub struct AppState {
    /// Database connection
    pub db: Arc<Database>,
    /// Configuration
    pub config: Arc<Config>,
    /// Bundle storage backend
    pub storage: Arc<dyn BundleStorage>,
    /// Bundle service for compilation and promotion
    pub bundle_service: Arc<BundleService>,
    /// Event broadcaster for SSE
    pub event_tx: broadcast::Sender<ServerEvent>,
    /// Server start time
    pub started_at: chrono::DateTime<chrono::Utc>,
    /// JWKS validator for external IdP tokens
    pub jwks_validator: Option<Arc<JwksValidator>>,
    /// ClickHouse-backed decision-log store (None until REAPER_CLICKHOUSE_URL is set)
    pub decision_store: Option<Arc<crate::decisions::DecisionStore>>,
    /// Shutdown signal for graceful shutdown
    shutdown_signal: ShutdownSignal,
    /// Flag indicating server is shutting down
    is_shutting_down: Arc<AtomicBool>,
}

impl AppState {
    /// Create new application state
    pub fn new(db: Arc<Database>, config: Config, storage: Arc<dyn BundleStorage>) -> Self {
        let (event_tx, _) = broadcast::channel(1024);
        // Build the bundle signer from config; an invalid key is logged and
        // signing stays off (compiled bundles will be unsigned, which agents
        // that require signatures will reject).
        let signer = match BundleSigner::from_config(&config.bundles) {
            Ok(Some(s)) => {
                tracing::info!(
                    key_id = %config.bundles.signing_key_id,
                    algorithm = %config.bundles.signing_algorithm,
                    "Bundle signing enabled"
                );
                Some(s)
            }
            Ok(None) => {
                tracing::warn!(
                    "No bundle signing key configured (REAPER_BUNDLE_SIGNING_KEY); \
                     compiled bundles will be UNSIGNED"
                );
                None
            }
            Err(e) => {
                tracing::error!(error = %e, "Invalid bundle signing key/algorithm; \
                    compiled bundles will be UNSIGNED");
                None
            }
        };
        let bundle_service =
            Arc::new(BundleService::new(db.clone(), storage.clone()).with_signer(signer));
        let jwks_validator = Arc::new(JwksValidator::new());
        let decision_store = crate::decisions::DecisionStore::from_env().map(Arc::new);
        if decision_store.is_some() {
            tracing::info!("Decision-log query API enabled (ClickHouse)");
        } else {
            tracing::info!("Decision-log query API disabled (set REAPER_CLICKHOUSE_URL to enable)");
        }

        Self {
            db,
            config: Arc::new(config),
            storage,
            bundle_service,
            event_tx,
            started_at: chrono::Utc::now(),
            jwks_validator: Some(jwks_validator),
            decision_store,
            shutdown_signal: ShutdownSignal::new(),
            is_shutting_down: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Get the shutdown signal for graceful shutdown coordination
    pub fn shutdown_signal(&self) -> &ShutdownSignal {
        &self.shutdown_signal
    }

    /// Check if the server is shutting down
    pub fn is_shutting_down(&self) -> bool {
        self.is_shutting_down.load(Ordering::SeqCst)
    }

    /// Initiate shutdown
    pub fn initiate_shutdown(&self) {
        self.is_shutting_down.store(true, Ordering::SeqCst);
        self.shutdown_signal.shutdown();
    }

    /// Get a new event receiver for SSE connections
    pub fn subscribe_events(&self) -> broadcast::Receiver<ServerEvent> {
        self.event_tx.subscribe()
    }

    /// Broadcast an event to all connected clients
    pub fn broadcast_event(&self, event: ServerEvent) {
        // Ignore send errors (no subscribers)
        let _ = self.event_tx.send(event);
    }

    /// Get server uptime in seconds
    pub fn uptime_seconds(&self) -> i64 {
        chrono::Utc::now()
            .signed_duration_since(self.started_at)
            .num_seconds()
    }
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("db", &self.db)
            .field("started_at", &self.started_at)
            .field("jwks_validator", &self.jwks_validator.is_some())
            .field("is_shutting_down", &self.is_shutting_down())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DatabaseConfig;
    use crate::storage::FilesystemStorage;

    #[tokio::test]
    async fn test_event_broadcast() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let storage_path = temp_dir.path().join("storage");
        std::fs::create_dir_all(&storage_path).unwrap();
        let url = format!("sqlite:{}", db_path.display());

        let db_config = DatabaseConfig {
            db_type: "sqlite".to_string(),
            url,
            max_connections: 5,
        };

        let db = Database::new(&db_config).await.unwrap();
        db.run_migrations().await.unwrap();
        let storage =
            Arc::new(FilesystemStorage::new(&storage_path).unwrap()) as Arc<dyn BundleStorage>;
        let state = AppState::new(Arc::new(db), Config::default(), storage);

        let mut rx = state.subscribe_events();

        state.broadcast_event(ServerEvent::Ping {
            timestamp: chrono::Utc::now(),
        });

        let event = rx.recv().await.unwrap();
        assert!(matches!(event, ServerEvent::Ping { .. }));
    }
}
