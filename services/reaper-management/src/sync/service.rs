//! Sync service for orchestrating policy synchronization
//!
//! Manages background sync tasks for all policy sources.

use std::path::PathBuf;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::broadcast;
use tokio::sync::RwLock;
use tokio::time::{interval, Duration};
use tracing::{debug, error, info, warn};

use crate::db::repositories::PolicySourceRepository;
use crate::db::Database;
use crate::domain::source::{PolicySource, SourceType, SyncResult, SyncStatus};
use crate::state::ServerEvent;

use super::api::{ApiSyncError, ApiSyncer};
use super::bundle_url::{BundleUrlSyncError, BundleUrlSyncer};
use super::git::{GitSyncError, GitSyncer};
use super::s3::{S3SyncError, S3Syncer};

/// Unified sync error
#[derive(Debug, Error)]
pub enum SyncError {
    #[error("Git sync error: {0}")]
    Git(#[from] GitSyncError),
    #[error("API sync error: {0}")]
    Api(#[from] ApiSyncError),
    #[error("S3 sync error: {0}")]
    S3(#[from] S3SyncError),
    #[error("Bundle URL sync error: {0}")]
    BundleUrl(#[from] BundleUrlSyncError),
    #[error("Database error: {0}")]
    Database(#[from] crate::db::DatabaseError),
    #[error("Source not found: {0}")]
    NotFound(String),
    #[error("Source cannot sync: {0}")]
    CannotSync(String),
}

/// Sync service configuration
#[derive(Debug, Clone)]
pub struct SyncConfig {
    /// Base path for Git repositories
    pub git_base_path: PathBuf,
    /// Base path for S3 cache
    pub s3_cache_path: PathBuf,
    /// Base path for bundle URL storage
    pub bundle_storage_path: PathBuf,
    /// Interval to check for due syncs
    pub check_interval_secs: u64,
    /// Maximum concurrent sync operations
    pub max_concurrent: usize,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            git_base_path: PathBuf::from("/tmp/reaper-sync/git"),
            s3_cache_path: PathBuf::from("/tmp/reaper-sync/s3"),
            bundle_storage_path: PathBuf::from("/tmp/reaper-sync/bundles"),
            check_interval_secs: 60,
            max_concurrent: 5,
        }
    }
}

/// Sync service for managing policy source synchronization
pub struct SyncService {
    config: SyncConfig,
    git_syncer: GitSyncer,
    api_syncer: ApiSyncer,
    s3_syncer: S3Syncer,
    bundle_url_syncer: BundleUrlSyncer,
    db: Arc<Database>,
    running: Arc<RwLock<bool>>,
    /// Optional event broadcaster for SSE notifications
    event_tx: Option<broadcast::Sender<ServerEvent>>,
}

impl SyncService {
    /// Create a new sync service
    pub fn new(db: Arc<Database>, config: SyncConfig) -> Self {
        let git_syncer = GitSyncer::new(&config.git_base_path);
        let api_syncer = ApiSyncer::new();
        let s3_syncer = S3Syncer::new(&config.s3_cache_path);
        let bundle_url_syncer = BundleUrlSyncer::new(&config.bundle_storage_path);

        Self {
            config,
            git_syncer,
            api_syncer,
            s3_syncer,
            bundle_url_syncer,
            db,
            running: Arc::new(RwLock::new(false)),
            event_tx: None,
        }
    }

    /// Create a new sync service with event broadcasting
    pub fn with_event_tx(
        db: Arc<Database>,
        config: SyncConfig,
        event_tx: broadcast::Sender<ServerEvent>,
    ) -> Self {
        let mut service = Self::new(db, config);
        service.event_tx = Some(event_tx);
        service
    }

    /// Broadcast an event if event_tx is configured
    fn broadcast(&self, event: ServerEvent) {
        if let Some(tx) = &self.event_tx {
            let _ = tx.send(event);
        }
    }

    /// Start the background sync loop
    pub async fn start(&self) {
        let mut running = self.running.write().await;
        if *running {
            warn!("Sync service already running");
            return;
        }
        *running = true;
        drop(running);

        info!(
            check_interval_secs = self.config.check_interval_secs,
            "Starting sync service"
        );

        let mut check_interval = interval(Duration::from_secs(self.config.check_interval_secs));

        loop {
            check_interval.tick().await;

            // Check if we should stop
            let running = self.running.read().await;
            if !*running {
                info!("Sync service stopping");
                break;
            }
            drop(running);

            // Find sources due for sync
            if let Err(e) = self.sync_due_sources().await {
                error!("Error syncing sources: {}", e);
            }
        }
    }

    /// Stop the background sync loop
    pub async fn stop(&self) {
        let mut running = self.running.write().await;
        *running = false;
        info!("Sync service stop requested");
    }

    /// Sync all sources that are due
    async fn sync_due_sources(&self) -> Result<(), SyncError> {
        let repo = PolicySourceRepository::new(&self.db);
        let due_sources = repo.list_due_for_sync().await?;

        if due_sources.is_empty() {
            debug!("No sources due for sync");
            return Ok(());
        }

        info!(count = due_sources.len(), "Found sources due for sync");

        // Sync each source (could be parallelized with semaphore)
        for source in due_sources {
            if let Err(e) = self.sync_source(&source).await {
                error!(source_id = %source.id, error = %e, "Failed to sync source");
            }
        }

        Ok(())
    }

    /// Sync a specific source
    pub async fn sync_source(&self, source: &PolicySource) -> Result<SyncResult, SyncError> {
        let repo = PolicySourceRepository::new(&self.db);

        // Mark as syncing
        repo.update_sync_status(source.id, SyncStatus::Syncing, None, None)
            .await?;

        // Emit sync started event
        self.broadcast(ServerEvent::SyncStarted {
            source_id: source.id,
            source_name: source.name.clone(),
            org_id: source.org_id,
            namespace_id: None, // Sources are org-wide, not namespace-scoped
        });

        // Perform sync based on source type
        let result: Result<SyncResult, SyncError> = match source.source_type {
            SourceType::Git => self.git_syncer.sync(source).await.map_err(SyncError::from),
            SourceType::Api => self.api_syncer.sync(source).await.map_err(SyncError::from),
            SourceType::S3 => self.s3_syncer.sync(source).await.map_err(SyncError::from),
            SourceType::BundleUrl => self
                .bundle_url_syncer
                .sync(source)
                .await
                .map_err(SyncError::from),
        };

        // Update status based on result
        match &result {
            Ok(sync_result) => {
                repo.update_sync_status(
                    source.id,
                    SyncStatus::Success,
                    None,
                    sync_result.commit.as_deref(),
                )
                .await?;

                info!(
                    source_id = %source.id,
                    source_name = %source.name,
                    policies_found = sync_result.policies_found,
                    "Source sync completed successfully"
                );

                // Emit sync completed event
                self.broadcast(ServerEvent::SyncCompleted {
                    source_id: source.id,
                    source_name: source.name.clone(),
                    org_id: source.org_id,
                    namespace_id: None, // Sources are org-wide, not namespace-scoped
                    policies_updated: sync_result.policies_found as u32,
                    duration_ms: sync_result.duration_ms,
                });
            }
            Err(e) => {
                let error_msg = e.to_string();
                repo.update_sync_status(source.id, SyncStatus::Failed, Some(&error_msg), None)
                    .await?;

                warn!(
                    source_id = %source.id,
                    source_name = %source.name,
                    error = %error_msg,
                    "Source sync failed"
                );

                // Emit sync failed event
                self.broadcast(ServerEvent::SyncFailed {
                    source_id: source.id,
                    source_name: source.name.clone(),
                    org_id: source.org_id,
                    namespace_id: None, // Sources are org-wide, not namespace-scoped
                    error: error_msg,
                });
            }
        }

        result
    }

    /// Manually trigger sync for a source
    pub async fn trigger_sync(&self, source_id: uuid::Uuid) -> Result<SyncResult, SyncError> {
        let repo = PolicySourceRepository::new(&self.db);

        let source = repo
            .get_by_id(source_id)
            .await?
            .ok_or_else(|| SyncError::NotFound(source_id.to_string()))?;

        if !source.can_sync() {
            return Err(SyncError::CannotSync(format!(
                "status: {}, enabled: {}",
                source.sync_status, source.is_enabled
            )));
        }

        self.sync_source(&source).await
    }

    /// Get the Git syncer (for reading policy files)
    pub fn git_syncer(&self) -> &GitSyncer {
        &self.git_syncer
    }

    /// Get the API syncer (for reading policies)
    pub fn api_syncer(&self) -> &ApiSyncer {
        &self.api_syncer
    }

    /// Get the S3 syncer (for reading policies from S3)
    pub fn s3_syncer(&self) -> &S3Syncer {
        &self.s3_syncer
    }

    /// Get the Bundle URL syncer (for fetching bundles)
    pub fn bundle_url_syncer(&self) -> &BundleUrlSyncer {
        &self.bundle_url_syncer
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

    #[tokio::test]
    async fn test_sync_service_creation() {
        let (_temp_dir, db) = setup_db().await;

        let config = SyncConfig {
            git_base_path: PathBuf::from("/tmp/test-sync/git"),
            s3_cache_path: PathBuf::from("/tmp/test-sync/s3"),
            bundle_storage_path: PathBuf::from("/tmp/test-sync/bundles"),
            check_interval_secs: 60,
            max_concurrent: 5,
        };

        let service = SyncService::new(db, config);
        assert!(!*service.running.read().await);
    }

    #[tokio::test]
    async fn test_sync_config_default() {
        let config = SyncConfig::default();
        assert_eq!(config.check_interval_secs, 60);
        assert_eq!(config.max_concurrent, 5);
        assert!(config.git_base_path.to_string_lossy().contains("git"));
        assert!(config.s3_cache_path.to_string_lossy().contains("s3"));
        assert!(config
            .bundle_storage_path
            .to_string_lossy()
            .contains("bundles"));
    }
}
