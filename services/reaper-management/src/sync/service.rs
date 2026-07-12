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

use crate::bundle::{BundleError, BundleService};
use crate::db::repositories::{BundleRepository, PolicyRepository, PolicySourceRepository};
use crate::db::Database;
use crate::domain::bundle::CreateBundle;
use crate::domain::policy::{CreatePolicy, PolicyLanguage, UpdatePolicy};
use crate::domain::source::{PolicySource, SourceType, SyncResult, SyncStatus};
use crate::state::ServerEvent;

use super::api::{ApiSyncError, ApiSyncer};
use super::bundle_url::{BundleUrlSyncError, BundleUrlSyncer};
use super::git::{GitSyncError, GitSyncer};
use super::github_app::GitHubAppClient;
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
    #[error("Bundle error: {0}")]
    Bundle(#[from] BundleError),
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
    /// Compile the materialized bundle at sync time (Plan 09 Step 2). Mirrors
    /// `bundles.auto_compile_on_source_sync`. When false, sync still upserts
    /// policies and creates the bundle (draft, linked to the commit SHA) but
    /// leaves compilation to the operator — promotion/rollout is always a
    /// separate, gated step either way.
    pub auto_compile: bool,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            git_base_path: PathBuf::from("/tmp/reaper-sync/git"),
            s3_cache_path: PathBuf::from("/tmp/reaper-sync/s3"),
            bundle_storage_path: PathBuf::from("/tmp/reaper-sync/bundles"),
            check_interval_secs: 60,
            max_concurrent: 5,
            auto_compile: false,
        }
    }
}

/// What a git-sync materialization produced (Plan 09 Step 2).
#[derive(Debug, Default)]
struct MaterializeOutcome {
    policies_created: usize,
    policies_updated: usize,
    bundle_id: Option<uuid::Uuid>,
}

/// Derive the org-unique policy name for a synced file. Namespaced by the
/// source name so two sources shipping the same path never collide, with the
/// extension stripped and separators flattened: source "prod-policies" +
/// "auth/rbac.reap" → "prod-policies/auth-rbac".
fn source_policy_name(source_name: &str, file_path: &str) -> String {
    let no_ext = std::path::Path::new(file_path)
        .with_extension("")
        .to_string_lossy()
        .replace(['/', '\\'], "-");
    format!("{source_name}/{no_ext}")
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
    /// Bundle service used to materialize synced policy files into a
    /// compiled bundle (Plan 09 Step 2). Without it, sync only counts files —
    /// the pre-Plan-09 behavior kept for unit tests.
    bundle_service: Option<Arc<BundleService>>,
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
            bundle_service: None,
        }
    }

    /// Attach the bundle service so successful git syncs materialize into
    /// policy rows + a bundle keyed by the commit SHA (Plan 09 Step 2).
    pub fn with_materializer(mut self, bundle_service: Arc<BundleService>) -> Self {
        self.bundle_service = Some(bundle_service);
        self
    }

    /// Attach a GitHub App client so App-installed sources clone with a
    /// short-lived installation token minted at sync time (Plan 09 Step 6).
    pub fn with_github_app(mut self, app_client: Option<Arc<GitHubAppClient>>) -> Self {
        self.git_syncer = GitSyncer::new(&self.config.git_base_path).with_app_client(app_client);
        self
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

        // Materialize git syncs into policy rows + a bundle (Plan 09 Step 2).
        // Counting files without persisting them was the F2 gap; a failed
        // materialization therefore fails the whole sync, not just a log line.
        let result: Result<SyncResult, SyncError> = match result {
            Ok(mut sync_result) if matches!(source.source_type, SourceType::Git) => {
                match self.materialize_git(source, &sync_result).await {
                    Ok(outcome) => {
                        sync_result.policies_created = outcome.policies_created;
                        sync_result.policies_updated = outcome.policies_updated;
                        Ok(sync_result)
                    }
                    Err(e) => Err(e),
                }
            }
            other => other,
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

    /// Materialize a successful git sync: upsert every synced policy file as
    /// a policy row and create a bundle linked to the commit SHA (Plan 09
    /// Step 2). Idempotent per SHA — a webhook and a poll landing on the same
    /// commit never double-apply. Compilation follows `config.auto_compile`;
    /// promotion/rollout to agents is always a separate, gated step.
    async fn materialize_git(
        &self,
        source: &PolicySource,
        sync_result: &SyncResult,
    ) -> Result<MaterializeOutcome, SyncError> {
        let mut outcome = MaterializeOutcome::default();

        let Some(bundle_service) = &self.bundle_service else {
            debug!(
                source_id = %source.id,
                "No materializer configured; sync counted files only"
            );
            return Ok(outcome);
        };
        let Some(commit) = sync_result.commit.as_deref() else {
            return Ok(outcome);
        };

        // Idempotency: this (source, SHA) pair was already materialized.
        let bundle_repo = BundleRepository::new(&self.db);
        if let Some(existing) = bundle_repo.find_by_source_commit(source.id, commit).await? {
            info!(
                source_id = %source.id,
                commit = %commit,
                bundle_id = %existing,
                "Commit already materialized; skipping (idempotent)"
            );
            outcome.bundle_id = Some(existing);
            return Ok(outcome);
        }

        let files = self.git_syncer.get_policy_files(source)?;
        if files.is_empty() {
            warn!(
                source_id = %source.id,
                commit = %commit,
                "Sync found no policy files; nothing to materialize"
            );
            return Ok(outcome);
        }

        let policy_repo = PolicyRepository::new(&self.db);
        let mut policy_ids = Vec::with_capacity(files.len());
        for file in &files {
            let name = source_policy_name(&source.name, &file.path);
            let language = match file.language.as_str() {
                "cedar" => PolicyLanguage::Cedar,
                "simple" => PolicyLanguage::Simple,
                _ => PolicyLanguage::Reaper,
            };

            match policy_repo.get_by_name(source.org_id, &name).await? {
                Some(existing) => {
                    // Policy names are unique per org; never let a git file
                    // silently take over a policy that a different source (or
                    // a hand edit) owns.
                    if existing.source_id != Some(source.id) {
                        return Err(SyncError::CannotSync(format!(
                            "policy '{}' already exists but is not owned by this source",
                            name
                        )));
                    }
                    let latest = policy_repo.get_latest_version(existing.id).await?;
                    let changed = latest.map(|v| v.content != file.content).unwrap_or(true);
                    if changed {
                        policy_repo
                            .update(
                                existing.id,
                                UpdatePolicy {
                                    content: Some(file.content.clone()),
                                    ..Default::default()
                                },
                                None,
                            )
                            .await?;
                        outcome.policies_updated += 1;
                    }
                    policy_ids.push(existing.id);
                }
                None => {
                    let created = policy_repo
                        .create(
                            source.org_id,
                            CreatePolicy {
                                name,
                                description: Some(format!(
                                    "Synced from source '{}' ({})",
                                    source.name, file.path
                                )),
                                team_id: None,
                                source_id: Some(source.id),
                                language,
                                source_path: Some(file.path.clone()),
                                content: file.content.clone(),
                            },
                        )
                        .await?;
                    outcome.policies_created += 1;
                    policy_ids.push(created.id);
                }
            }
        }

        // One bundle per (source, SHA); bundles are UNIQUE(org, name, version)
        // so the short SHA in the name also guards against duplicates.
        let short_sha = &commit[..commit.len().min(12)];
        let bundle = bundle_service
            .create(
                source.org_id,
                &CreateBundle {
                    name: format!("{}@{}", source.name, short_sha),
                    description: Some(format!(
                        "Materialized from git sync of '{}' at commit {}",
                        source.name, commit
                    )),
                    policy_ids: policy_ids.clone(),
                },
            )
            .await?;
        bundle_repo
            .link_source(bundle.id, source.id, commit)
            .await?;

        if self.config.auto_compile {
            bundle_service.compile(bundle.id).await?;
            info!(
                source_id = %source.id,
                bundle_id = %bundle.id,
                commit = %commit,
                "Materialized bundle compiled (auto_compile_on_source_sync)"
            );
        }

        info!(
            source_id = %source.id,
            bundle_id = %bundle.id,
            commit = %commit,
            policies = policy_ids.len(),
            created = outcome.policies_created,
            updated = outcome.policies_updated,
            "Git sync materialized into policies + bundle"
        );
        outcome.bundle_id = Some(bundle.id);
        Ok(outcome)
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
    use tempfile::TempDir;

    async fn setup_db() -> (TempDir, Arc<Database>) {
        let temp_dir = TempDir::new().unwrap();

        let config = crate::db::ephemeral_test_config(temp_dir.path()).await;

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
            auto_compile: false,
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
