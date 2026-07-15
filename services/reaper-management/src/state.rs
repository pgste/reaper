//! Application state for Reaper Management Server
//!
//! Holds shared state accessible to all request handlers.

use crate::auth::JwksValidator;
use crate::bundle::{BundleService, BundleSigner};
use crate::config::Config;
use crate::db::Database;
use crate::graceful::ShutdownSignal;
use crate::storage::BundleStorage;
use crate::sync::{SyncConfig as SyncServiceConfig, SyncService};
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
    /// Drift detected between a git source's HEAD and its deployed policies
    /// (Plan 09 Step 8).
    DriftDetected {
        source_id: uuid::Uuid,
        source_name: String,
        org_id: uuid::Uuid,
        namespace_id: Option<uuid::Uuid>,
        added: usize,
        removed: usize,
        changed: usize,
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
    /// The rollout supervisor's auto-rollback trigger fired for a rollout
    /// (B2 / PROD R2-1). `enforced` is false in monitor mode (alert only);
    /// when true, `rollback_rollout_id` names the remediation rollout the
    /// supervisor started.
    AutoRollbackTriggered {
        rollout_id: uuid::Uuid,
        org_id: uuid::Uuid,
        namespace_id: Option<uuid::Uuid>,
        error_rate: f64,
        threshold: f64,
        enforced: bool,
        rollback_rollout_id: Option<uuid::Uuid>,
    },
    /// A datastore version was published (data plane): agents fetch the
    /// materialized document for `version` and hot-swap their DataStore.
    DatastorePublished {
        datastore_id: uuid::Uuid,
        org_id: uuid::Uuid,
        namespace_id: Option<uuid::Uuid>,
        version: i64,
        checksum: String,
    },
    /// An agent's replicated authorization data crossed its staleness
    /// budget (self-reported via heartbeat; emitted on the transition,
    /// not every heartbeat).
    AgentDataStale {
        agent_id: uuid::Uuid,
        agent_name: String,
        org_id: uuid::Uuid,
        namespace_id: Option<uuid::Uuid>,
        data_version: i64,
        data_applied_seq: i64,
    },
    /// The agent's data replica caught back up (stale → fresh transition).
    AgentDataFresh {
        agent_id: uuid::Uuid,
        agent_name: String,
        org_id: uuid::Uuid,
        namespace_id: Option<uuid::Uuid>,
        data_version: i64,
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
            ServerEvent::DriftDetected { org_id, .. } => Some(*org_id),
            ServerEvent::AgentRegistered { org_id, .. } => Some(*org_id),
            ServerEvent::AgentUnhealthy { org_id, .. } => Some(*org_id),
            ServerEvent::AgentHealthy { org_id, .. } => Some(*org_id),
            ServerEvent::RolloutStarted { org_id, .. } => Some(*org_id),
            ServerEvent::RolloutWaveCompleted { org_id, .. } => Some(*org_id),
            ServerEvent::RolloutCompleted { org_id, .. } => Some(*org_id),
            ServerEvent::AutoRollbackTriggered { org_id, .. } => Some(*org_id),
            ServerEvent::DatastorePublished { org_id, .. } => Some(*org_id),
            ServerEvent::AgentDataStale { org_id, .. } => Some(*org_id),
            ServerEvent::AgentDataFresh { org_id, .. } => Some(*org_id),
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
            ServerEvent::DriftDetected { namespace_id, .. } => *namespace_id,
            ServerEvent::AgentRegistered { namespace_id, .. } => *namespace_id,
            ServerEvent::AgentUnhealthy { namespace_id, .. } => *namespace_id,
            ServerEvent::AgentHealthy { namespace_id, .. } => *namespace_id,
            ServerEvent::RolloutStarted { namespace_id, .. } => *namespace_id,
            ServerEvent::RolloutWaveCompleted { namespace_id, .. } => *namespace_id,
            ServerEvent::RolloutCompleted { namespace_id, .. } => *namespace_id,
            ServerEvent::AutoRollbackTriggered { namespace_id, .. } => *namespace_id,
            ServerEvent::DatastorePublished { namespace_id, .. } => *namespace_id,
            ServerEvent::AgentDataStale { namespace_id, .. } => *namespace_id,
            ServerEvent::AgentDataFresh { namespace_id, .. } => *namespace_id,
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
    /// Policy-source sync engine (Plan 09): spawned as a background loop at
    /// boot and invoked directly by the manual-trigger API.
    pub sync_service: Arc<SyncService>,
    /// Event broadcaster for SSE
    pub event_tx: broadcast::Sender<ServerEvent>,
    /// Server start time
    pub started_at: chrono::DateTime<chrono::Utc>,
    /// JWKS validator for external IdP tokens
    pub jwks_validator: Option<Arc<JwksValidator>>,
    /// ClickHouse-backed decision-log store (None until REAPER_CLICKHOUSE_URL is set)
    pub decision_store: Option<Arc<crate::decisions::DecisionStore>>,
    /// In-memory counterfactual-replay job registry (Plan 04 step 8).
    pub replay_jobs: crate::replay::ReplayJobs,
    /// Per-tenant request ceiling (round-2 E4): enforces `api_per_org_per_minute`
    /// on the resource-creating paths so one org cannot exhaust the shared
    /// control plane. `None` when rate limiting is disabled.
    pub org_rate_limiter: Option<Arc<crate::rate_limit::OrgRateLimiter>>,
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

        // GitHub App client (Plan 09 Step 6): minted-per-sync installation
        // tokens replace PAT-in-URL cloning. None unless the App is fully
        // configured (app_id + private key); an invalid key is logged and
        // App auth stays off, falling back to configured userpass.
        let github_app = config.oauth.github.as_ref().and_then(|gh| {
            match crate::sync::GitHubAppClient::from_config(
                gh.app_id.as_deref(),
                gh.app_private_key.as_deref(),
            ) {
                Ok(client) => {
                    tracing::info!("GitHub App auth enabled (installation tokens)");
                    Some(Arc::new(client))
                }
                Err(crate::sync::GitHubAppError::NotConfigured) => None,
                Err(e) => {
                    tracing::error!(error = %e, "Invalid GitHub App config; \
                            App auth disabled (falling back to userpass)");
                    None
                }
            }
        });

        // Sync engine (Plan 09): shares the SSE broadcaster so syncs surface
        // as events, and the bundle service so git syncs materialize into
        // policy rows + a bundle instead of only counting files (F2).
        let sync_service = Arc::new(
            SyncService::with_event_tx(
                db.clone(),
                SyncServiceConfig {
                    git_base_path: config.sync.git_base_path.clone(),
                    s3_cache_path: config.sync.s3_cache_path.clone(),
                    bundle_storage_path: config.sync.bundle_storage_path.clone(),
                    check_interval_secs: config.sync.check_interval_secs,
                    max_concurrent: config.sync.max_concurrent,
                    auto_compile: config.bundles.auto_compile_on_source_sync,
                },
                event_tx.clone(),
            )
            .with_materializer(bundle_service.clone())
            .with_github_app(github_app),
        );

        // Per-tenant request ceiling (E4): built only when rate limiting is on.
        let org_rate_limiter = config.rate_limit.enabled.then(|| {
            Arc::new(crate::rate_limit::OrgRateLimiter::new(
                config.rate_limit.api_per_org_per_minute,
            ))
        });

        Self {
            db,
            config: Arc::new(config),
            storage,
            bundle_service,
            sync_service,
            event_tx,
            started_at: chrono::Utc::now(),
            jwks_validator: Some(jwks_validator),
            decision_store,
            replay_jobs: std::sync::Arc::new(dashmap::DashMap::new()),
            org_rate_limiter,
            shutdown_signal: ShutdownSignal::new(),
            is_shutting_down: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Enforce the per-tenant request ceiling for `org_id` (E4). No-op when rate
    /// limiting is disabled; returns `false` when the org's per-minute ceiling is
    /// exhausted.
    pub fn allow_org_request(&self, org_id: uuid::Uuid) -> bool {
        self.org_rate_limiter
            .as_ref()
            .map(|l| l.allow(org_id))
            .unwrap_or(true)
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
    use crate::storage::FilesystemStorage;

    #[tokio::test]
    async fn test_event_broadcast() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let storage_path = temp_dir.path().join("storage");
        std::fs::create_dir_all(&storage_path).unwrap();

        let db_config = crate::db::ephemeral_test_config(temp_dir.path()).await;

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
