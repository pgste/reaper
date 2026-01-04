//! Bundle service for managing compilation and promotion
//!
//! Orchestrates bundle compilation, storage, and status transitions.

use std::sync::Arc;
use thiserror::Error;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::db::repositories::{BundleRepository, PolicyRepository};
use crate::db::Database;
use crate::domain::bundle::{Bundle, BundleStatus, CreateBundle, PromotionRequest};
use crate::storage::{BundleMetadata, BundleStorage, StorageError};

use super::compiler::{BundleCompiler, CompileError};

/// Bundle service errors
#[derive(Debug, Error)]
pub enum BundleError {
    #[error("Bundle not found: {0}")]
    NotFound(String),
    #[error("Invalid state transition: cannot {0} from {1}")]
    InvalidTransition(String, String),
    #[error("Compilation error: {0}")]
    Compilation(#[from] CompileError),
    #[error("Storage error: {0}")]
    Storage(#[from] StorageError),
    #[error("Database error: {0}")]
    Database(#[from] crate::db::DatabaseError),
    #[error("No policies in bundle")]
    NoPolicies,
    #[error("Validation error: {0}")]
    Validation(String),
}

/// Bundle service for managing compilation and promotion workflow
pub struct BundleService {
    db: Arc<Database>,
    storage: Arc<dyn BundleStorage>,
    compiler: BundleCompiler,
}

impl BundleService {
    /// Create a new bundle service
    pub fn new(db: Arc<Database>, storage: Arc<dyn BundleStorage>) -> Self {
        Self {
            db,
            storage,
            compiler: BundleCompiler::new(),
        }
    }

    /// Create a new bundle
    pub async fn create(&self, org_id: Uuid, input: &CreateBundle) -> Result<Bundle, BundleError> {
        let repo = BundleRepository::new(&self.db);
        let bundle = repo.create(org_id, input).await?;
        info!(bundle_id = %bundle.id, name = %bundle.name, "Bundle created");
        Ok(bundle)
    }

    /// Get a bundle by ID
    pub async fn get(&self, bundle_id: Uuid) -> Result<Bundle, BundleError> {
        let repo = BundleRepository::new(&self.db);
        repo.get_by_id(bundle_id)
            .await?
            .ok_or_else(|| BundleError::NotFound(bundle_id.to_string()))
    }

    /// List bundles for an organization
    pub async fn list(
        &self,
        org_id: Uuid,
        status_filter: Option<BundleStatus>,
    ) -> Result<Vec<Bundle>, BundleError> {
        let repo = BundleRepository::new(&self.db);
        Ok(repo.list_by_org(org_id, status_filter).await?)
    }

    /// Get the currently promoted bundle for an organization
    pub async fn get_promoted(&self, org_id: Uuid) -> Result<Option<Bundle>, BundleError> {
        let repo = BundleRepository::new(&self.db);
        Ok(repo.get_promoted(org_id).await?)
    }

    /// Delete a bundle
    pub async fn delete(&self, bundle_id: Uuid) -> Result<(), BundleError> {
        let repo = BundleRepository::new(&self.db);

        // Get the bundle first to check if we need to delete from storage
        let bundle = self.get(bundle_id).await?;

        // Delete from storage if compiled
        if let Some(storage_key) = &bundle.storage_key {
            if let Err(e) = self.storage.delete(storage_key).await {
                warn!(
                    bundle_id = %bundle_id,
                    storage_key = %storage_key,
                    error = %e,
                    "Failed to delete bundle from storage"
                );
                // Continue with database deletion even if storage deletion fails
            }
        }

        repo.delete(bundle_id).await?;
        info!(bundle_id = %bundle_id, "Bundle deleted");
        Ok(())
    }

    /// Add policies to a bundle
    pub async fn add_policies(
        &self,
        bundle_id: Uuid,
        policy_ids: &[Uuid],
    ) -> Result<Bundle, BundleError> {
        let bundle = self.get(bundle_id).await?;

        // Can only modify draft bundles
        if bundle.status != BundleStatus::Draft {
            return Err(BundleError::InvalidTransition(
                "add policies".to_string(),
                bundle.status.to_string(),
            ));
        }

        let repo = BundleRepository::new(&self.db);
        for (idx, policy_id) in policy_ids.iter().enumerate() {
            repo.add_policy(bundle_id, *policy_id, idx as i32).await?;
        }

        self.get(bundle_id).await
    }

    /// Remove policies from a bundle
    pub async fn remove_policies(
        &self,
        bundle_id: Uuid,
        policy_ids: &[Uuid],
    ) -> Result<Bundle, BundleError> {
        let bundle = self.get(bundle_id).await?;

        // Can only modify draft bundles
        if bundle.status != BundleStatus::Draft {
            return Err(BundleError::InvalidTransition(
                "remove policies".to_string(),
                bundle.status.to_string(),
            ));
        }

        let repo = BundleRepository::new(&self.db);
        for policy_id in policy_ids {
            repo.remove_policy(bundle_id, *policy_id).await?;
        }

        self.get(bundle_id).await
    }

    /// Compile a bundle
    pub async fn compile(&self, bundle_id: Uuid) -> Result<Bundle, BundleError> {
        let bundle = self.get(bundle_id).await?;

        // Check if compilation is allowed
        if !bundle.can_compile() {
            return Err(BundleError::InvalidTransition(
                "compile".to_string(),
                bundle.status.to_string(),
            ));
        }

        let bundle_repo = BundleRepository::new(&self.db);
        let policy_repo = PolicyRepository::new(&self.db);

        // Get policies in the bundle
        let bundle_policies = bundle_repo.get_policies(bundle_id).await?;
        if bundle_policies.is_empty() {
            return Err(BundleError::NoPolicies);
        }

        // Get policy versions
        let mut policy_versions = Vec::new();
        for bp in &bundle_policies {
            if let Some(pv) = policy_repo.get_version(bp.policy_id, bp.policy_version).await? {
                policy_versions.push(pv);
            }
        }

        // Compile the bundle
        debug!(bundle_id = %bundle_id, policies = bundle_policies.len(), "Compiling bundle");
        let compiled = self.compiler.compile(&bundle_policies, &policy_versions)?;

        // Store the compiled bundle
        let storage_key = format!("bundles/{}/{}.rbb", bundle.org_id, bundle_id);
        let metadata = BundleMetadata::new(
            bundle.org_id,
            bundle_id,
            "1.0.0".to_string(),
            compiled.policy_count as usize,
            compiled.checksum.clone(),
        );

        self.storage
            .put(&storage_key, &compiled.data, metadata)
            .await?;

        // Update bundle record
        let updated = bundle_repo
            .update_compilation(
                bundle_id,
                &storage_key,
                compiled.data.len() as i64,
                &compiled.checksum,
                compiled.policy_count,
            )
            .await?;

        info!(
            bundle_id = %bundle_id,
            storage_key = %storage_key,
            size_bytes = compiled.data.len(),
            policy_count = compiled.policy_count,
            "Bundle compiled and stored"
        );

        Ok(updated)
    }

    /// Stage a bundle for testing
    pub async fn stage(&self, bundle_id: Uuid) -> Result<Bundle, BundleError> {
        let bundle = self.get(bundle_id).await?;

        if !bundle.can_stage() {
            return Err(BundleError::InvalidTransition(
                "stage".to_string(),
                bundle.status.to_string(),
            ));
        }

        let repo = BundleRepository::new(&self.db);
        let updated = repo
            .update_status(bundle_id, BundleStatus::Staged, None, Some("Staged for testing"))
            .await?;

        info!(bundle_id = %bundle_id, "Bundle staged");
        Ok(updated)
    }

    /// Promote a bundle to production
    pub async fn promote(
        &self,
        bundle_id: Uuid,
        request: &PromotionRequest,
    ) -> Result<Bundle, BundleError> {
        let bundle = self.get(bundle_id).await?;

        if !bundle.can_promote() {
            return Err(BundleError::InvalidTransition(
                "promote".to_string(),
                bundle.status.to_string(),
            ));
        }

        let repo = BundleRepository::new(&self.db);

        // Archive previously promoted bundle if exists
        if let Some(current_promoted) = repo.get_promoted(bundle.org_id).await? {
            if current_promoted.id != bundle_id {
                repo.update_status(
                    current_promoted.id,
                    BundleStatus::Deprecated,
                    None,
                    Some("Replaced by newer bundle"),
                )
                .await?;
                info!(
                    old_bundle_id = %current_promoted.id,
                    new_bundle_id = %bundle_id,
                    "Previous bundle deprecated"
                );
            }
        }

        // Promote the new bundle
        let updated = repo
            .update_status(
                bundle_id,
                BundleStatus::Promoted,
                None,
                request.notes.as_deref(),
            )
            .await?;

        info!(
            bundle_id = %bundle_id,
            notify_only = request.notify_only,
            "Bundle promoted to production"
        );

        // TODO: If not notify_only, trigger deployment to agents
        // This would integrate with the SSE event system

        Ok(updated)
    }

    /// Deprecate a bundle
    pub async fn deprecate(&self, bundle_id: Uuid, notes: Option<&str>) -> Result<Bundle, BundleError> {
        let bundle = self.get(bundle_id).await?;

        if !bundle.can_deprecate() {
            return Err(BundleError::InvalidTransition(
                "deprecate".to_string(),
                bundle.status.to_string(),
            ));
        }

        let repo = BundleRepository::new(&self.db);
        let updated = repo
            .update_status(bundle_id, BundleStatus::Deprecated, None, notes)
            .await?;

        info!(bundle_id = %bundle_id, "Bundle deprecated");
        Ok(updated)
    }

    /// Get the compiled bundle data
    pub async fn download(&self, bundle_id: Uuid) -> Result<Vec<u8>, BundleError> {
        let bundle = self.get(bundle_id).await?;

        let storage_key = bundle
            .storage_key
            .as_ref()
            .ok_or_else(|| BundleError::Validation("Bundle not compiled".to_string()))?;

        let stored = self
            .storage
            .get(storage_key)
            .await?
            .ok_or_else(|| BundleError::NotFound(format!("Bundle data not found: {}", storage_key)))?;

        debug!(
            bundle_id = %bundle_id,
            size_bytes = stored.data.len(),
            "Bundle downloaded"
        );

        Ok(stored.data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DatabaseConfig;
    use crate::storage::FilesystemStorage;
    use sha2::Digest;
    use tempfile::TempDir;

    async fn setup() -> (TempDir, Arc<Database>, Arc<dyn BundleStorage>) {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let storage_path = temp_dir.path().join("storage");
        std::fs::create_dir_all(&storage_path).unwrap();

        let db_config = DatabaseConfig {
            db_type: "sqlite".to_string(),
            url: format!("sqlite:{}", db_path.display()),
            max_connections: 5,
        };

        let db = Database::new(&db_config).await.unwrap();
        db.run_migrations().await.unwrap();

        let storage = Arc::new(FilesystemStorage::new(&storage_path).unwrap()) as Arc<dyn BundleStorage>;

        (temp_dir, Arc::new(db), storage)
    }

    async fn create_test_org(db: &Database) -> Uuid {
        let pool = db.sqlite_pool().unwrap();
        let org_id = Uuid::new_v4();
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO organizations (id, name, slug, created_at, updated_at) VALUES (?, ?, ?, ?, ?)"
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

    async fn create_test_policy(db: &Database, org_id: Uuid, name: &str) -> Uuid {
        let pool = db.sqlite_pool().unwrap();
        let policy_id = Uuid::new_v4();
        let version_id = Uuid::new_v4();
        let now = chrono::Utc::now().to_rfc3339();
        let content = format!("policy content for {}", name);
        let content_hash = format!("{:x}", sha2::Sha256::digest(content.as_bytes()));

        sqlx::query(
            "INSERT INTO policies (id, org_id, name, language, current_version, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?)"
        )
        .bind(policy_id.to_string())
        .bind(org_id.to_string())
        .bind(name)
        .bind("reaper")
        .bind(1)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO policy_versions (id, policy_id, version, content, content_hash, created_at) VALUES (?, ?, ?, ?, ?, ?)"
        )
        .bind(version_id.to_string())
        .bind(policy_id.to_string())
        .bind(1)
        .bind(&content)
        .bind(&content_hash)
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();

        policy_id
    }

    #[tokio::test]
    async fn test_create_bundle() {
        let (_temp_dir, db, storage) = setup().await;
        let org_id = create_test_org(&db).await;

        let service = BundleService::new(db.clone(), storage);

        let input = CreateBundle {
            name: "test-bundle".to_string(),
            description: Some("Test bundle".to_string()),
            policy_ids: vec![],
        };

        let bundle = service.create(org_id, &input).await.unwrap();
        assert_eq!(bundle.name, "test-bundle");
        assert_eq!(bundle.status, BundleStatus::Draft);
    }

    #[tokio::test]
    async fn test_compile_bundle() {
        let (_temp_dir, db, storage) = setup().await;
        let org_id = create_test_org(&db).await;
        let policy_id = create_test_policy(&db, org_id, "test-policy").await;

        let service = BundleService::new(db.clone(), storage);

        // Create bundle with a policy
        let input = CreateBundle {
            name: "compile-test".to_string(),
            description: None,
            policy_ids: vec![policy_id],
        };

        let bundle = service.create(org_id, &input).await.unwrap();
        assert_eq!(bundle.policy_count, 1);

        // Compile the bundle
        let compiled = service.compile(bundle.id).await.unwrap();
        assert_eq!(compiled.status, BundleStatus::Compiled);
        assert!(compiled.storage_key.is_some());
        assert!(compiled.checksum.is_some());
    }

    #[tokio::test]
    async fn test_promotion_workflow() {
        let (_temp_dir, db, storage) = setup().await;
        let org_id = create_test_org(&db).await;
        let policy_id = create_test_policy(&db, org_id, "workflow-policy").await;

        let service = BundleService::new(db.clone(), storage);

        // Create and compile bundle
        let input = CreateBundle {
            name: "workflow-test".to_string(),
            description: None,
            policy_ids: vec![policy_id],
        };

        let bundle = service.create(org_id, &input).await.unwrap();
        let bundle = service.compile(bundle.id).await.unwrap();
        assert_eq!(bundle.status, BundleStatus::Compiled);

        // Stage the bundle
        let bundle = service.stage(bundle.id).await.unwrap();
        assert_eq!(bundle.status, BundleStatus::Staged);

        // Promote the bundle
        let bundle = service
            .promote(bundle.id, &PromotionRequest::default())
            .await
            .unwrap();
        assert_eq!(bundle.status, BundleStatus::Promoted);

        // Deprecate the bundle
        let bundle = service.deprecate(bundle.id, Some("Testing")).await.unwrap();
        assert_eq!(bundle.status, BundleStatus::Deprecated);
    }

    #[tokio::test]
    async fn test_invalid_transition() {
        let (_temp_dir, db, storage) = setup().await;
        let org_id = create_test_org(&db).await;

        let service = BundleService::new(db.clone(), storage);

        let input = CreateBundle {
            name: "invalid-test".to_string(),
            description: None,
            policy_ids: vec![],
        };

        let bundle = service.create(org_id, &input).await.unwrap();

        // Cannot stage a draft bundle
        let result = service.stage(bundle.id).await;
        assert!(matches!(result, Err(BundleError::InvalidTransition(_, _))));

        // Cannot promote a draft bundle
        let result = service
            .promote(bundle.id, &PromotionRequest::default())
            .await;
        assert!(matches!(result, Err(BundleError::InvalidTransition(_, _))));
    }
}
