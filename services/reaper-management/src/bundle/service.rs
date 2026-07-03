//! Bundle service for managing compilation and promotion
//!
//! Orchestrates bundle compilation, storage, and status transitions.

use std::sync::Arc;
use thiserror::Error;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::config::BundlesConfig;
use crate::db::repositories::{BundleRepository, PolicyRepository};
use crate::db::Database;
use crate::domain::bundle::{Bundle, BundleStatus, CreateBundle, PromotionRequest};
use crate::storage::{BundleMetadata, BundleStorage, StorageError};
use reaper_core::bundle_signing::{self, BundleSignature, SigAlgorithm, SigningKey};

use super::compiler::{BundleCompiler, CompileError};

/// Suffix of the sidecar object holding a bundle's detached signature. Stored
/// next to the bundle in the same backend so it travels with it (e.g. to S3).
pub const SIGNATURE_SUFFIX: &str = ".sig";

/// Holds the control plane's private signing key and its advertised key id.
pub struct BundleSigner {
    key: SigningKey,
    key_id: String,
}

impl BundleSigner {
    /// Build a signer from bundle config. Returns `Ok(None)` when no signing key
    /// is configured (signing disabled), or an error if the configured
    /// key/algorithm is invalid.
    pub fn from_config(cfg: &BundlesConfig) -> Result<Option<Arc<Self>>, String> {
        let Some(hex) = cfg.signing_key.as_deref() else {
            return Ok(None);
        };
        let alg = SigAlgorithm::parse(&cfg.signing_algorithm).map_err(|e| e.to_string())?;
        let key = SigningKey::from_hex(alg, hex).map_err(|e| e.to_string())?;
        Ok(Some(Arc::new(Self {
            key,
            key_id: cfg.signing_key_id.clone(),
        })))
    }

    fn sign(&self, bytes: &[u8]) -> BundleSignature {
        bundle_signing::sign_bundle(bytes, &self.key, &self.key_id)
    }
}

/// Result of downloading a bundle: the bytes plus the detached signature
/// (if the bundle was signed at creation).
pub struct BundleDownloadResult {
    pub data: Vec<u8>,
    pub signature: Option<BundleSignature>,
}

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
    #[error("Signing error: {0}")]
    Signing(String),
}

/// Bundle service for managing compilation and promotion workflow
pub struct BundleService {
    db: Arc<Database>,
    storage: Arc<dyn BundleStorage>,
    compiler: BundleCompiler,
    /// Optional signer; when set, every compiled bundle is signed at creation.
    signer: Option<Arc<BundleSigner>>,
}

impl BundleService {
    /// Create a new bundle service (no bundle signing).
    pub fn new(db: Arc<Database>, storage: Arc<dyn BundleStorage>) -> Self {
        Self {
            db,
            storage,
            compiler: BundleCompiler::new(),
            signer: None,
        }
    }

    /// Attach a bundle signer so compiled bundles are signed at creation.
    pub fn with_signer(mut self, signer: Option<Arc<BundleSigner>>) -> Self {
        self.signer = signer;
        self
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
            // Best-effort removal of the signature sidecar.
            let sig_key = format!("{storage_key}{SIGNATURE_SUFFIX}");
            if let Err(e) = self.storage.delete(&sig_key).await {
                debug!(bundle_id = %bundle_id, error = %e,
                    "Failed to delete bundle signature sidecar (may not exist)");
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
            if let Some(pv) = policy_repo
                .get_version(bp.policy_id, bp.policy_version)
                .await?
            {
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

        // Sign at creation and store the signature as a sidecar object next to
        // the bundle, so it travels with the bundle to any store (S3, fs) and is
        // available whether the agent pulls from the control plane or directly.
        if let Some(signer) = &self.signer {
            let signature = signer.sign(&compiled.data);
            let sig_bytes = serde_json::to_vec(&signature)
                .map_err(|e| BundleError::Signing(format!("serialize signature: {e}")))?;
            let sig_key = format!("{storage_key}{SIGNATURE_SUFFIX}");
            let sig_metadata = BundleMetadata::new(
                bundle.org_id,
                bundle_id,
                "1.0.0".to_string(),
                compiled.policy_count as usize,
                signature.sha256.clone(),
            )
            .with_content_type("application/json");
            self.storage.put(&sig_key, &sig_bytes, sig_metadata).await?;
            info!(
                bundle_id = %bundle_id,
                key_id = %signature.key_id,
                algorithm = %signature.algorithm,
                "Bundle signed at creation"
            );
        } else {
            warn!(
                bundle_id = %bundle_id,
                "Bundle compiled WITHOUT a signature (no signing key configured); \
                 agents that require signed bundles will reject it"
            );
        }

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
            .update_status(
                bundle_id,
                BundleStatus::Staged,
                None,
                Some("Staged for testing"),
            )
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
    pub async fn deprecate(
        &self,
        bundle_id: Uuid,
        notes: Option<&str>,
    ) -> Result<Bundle, BundleError> {
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
    pub async fn download(&self, bundle_id: Uuid) -> Result<BundleDownloadResult, BundleError> {
        let bundle = self.get(bundle_id).await?;

        let storage_key = bundle
            .storage_key
            .as_ref()
            .ok_or_else(|| BundleError::Validation("Bundle not compiled".to_string()))?;

        let stored = self.storage.get(storage_key).await?.ok_or_else(|| {
            BundleError::NotFound(format!("Bundle data not found: {}", storage_key))
        })?;

        // Read the detached signature sidecar if present.
        let sig_key = format!("{storage_key}{SIGNATURE_SUFFIX}");
        let signature = match self.storage.get(&sig_key).await? {
            Some(sig_obj) => match serde_json::from_slice::<BundleSignature>(&sig_obj.data) {
                Ok(sig) => Some(sig),
                Err(e) => {
                    warn!(bundle_id = %bundle_id, error = %e,
                        "Bundle signature sidecar is malformed; serving unsigned");
                    None
                }
            },
            None => None,
        };

        debug!(
            bundle_id = %bundle_id,
            size_bytes = stored.data.len(),
            signed = signature.is_some(),
            "Bundle downloaded"
        );

        Ok(BundleDownloadResult {
            data: stored.data,
            signature,
        })
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

        let storage =
            Arc::new(FilesystemStorage::new(&storage_path).unwrap()) as Arc<dyn BundleStorage>;

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

        // Unsigned service -> no signature sidecar on download.
        let download = service.download(bundle.id).await.unwrap();
        assert!(download.signature.is_none());
    }

    fn signing_cfg(alg: &str) -> BundlesConfig {
        BundlesConfig {
            signing_key: Some("07".repeat(32)),
            signing_key_id: "k1".to_string(),
            signing_algorithm: alg.to_string(),
            ..Default::default()
        }
    }

    /// End-to-end: the control plane signs at compile time, stores the sidecar,
    /// serves it on download, and the AGENT-side primitive verifies it with the
    /// signer's public key. Runs for both algorithms.
    async fn sign_compile_download_verify(alg: &str, sig_alg: SigAlgorithm) {
        let (_temp_dir, db, storage) = setup().await;
        let org_id = create_test_org(&db).await;
        let policy_id = create_test_policy(&db, org_id, "p").await;

        let signer = BundleSigner::from_config(&signing_cfg(alg))
            .unwrap()
            .unwrap();
        let pub_hex = signer.key.public_key_hex();
        let service = BundleService::new(db.clone(), storage).with_signer(Some(signer));

        let input = CreateBundle {
            name: "signed".to_string(),
            description: None,
            policy_ids: vec![policy_id],
        };
        let bundle = service.create(org_id, &input).await.unwrap();
        service.compile(bundle.id).await.unwrap();

        // Download carries the signature.
        let download = service.download(bundle.id).await.unwrap();
        let sig = download.signature.expect("bundle must be signed");
        assert_eq!(sig.algorithm, alg);
        assert_eq!(sig.key_id, "k1");

        // Agent side: verify against the signer's public key. Tampering fails.
        let vk = reaper_core::bundle_signing::VerifyingKey::from_hex(sig_alg, &pub_hex).unwrap();
        reaper_core::bundle_signing::verify_bundle(&download.data, &sig, &vk, Some("k1")).unwrap();

        let mut tampered = download.data.clone();
        tampered.push(0xff);
        assert!(
            reaper_core::bundle_signing::verify_bundle(&tampered, &sig, &vk, Some("k1")).is_err()
        );
    }

    #[tokio::test]
    async fn test_signed_bundle_ed25519_verifies_on_agent_side() {
        sign_compile_download_verify(
            reaper_core::bundle_signing::ALG_ED25519,
            SigAlgorithm::Ed25519Sha256,
        )
        .await;
    }

    #[tokio::test]
    async fn test_signed_bundle_p256_verifies_on_agent_side() {
        sign_compile_download_verify(
            reaper_core::bundle_signing::ALG_ECDSA_P256,
            SigAlgorithm::EcdsaP256Sha256,
        )
        .await;
    }

    #[test]
    fn test_no_signing_key_means_no_signer() {
        assert!(BundleSigner::from_config(&BundlesConfig::default())
            .unwrap()
            .is_none());
    }

    #[test]
    fn test_bad_signing_key_errors() {
        let mut cfg = signing_cfg(reaper_core::bundle_signing::ALG_ED25519);
        cfg.signing_key = Some("nothex".to_string());
        assert!(BundleSigner::from_config(&cfg).is_err());
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
