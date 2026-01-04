//! Filesystem storage backend
//!
//! Stores bundles on the local filesystem with metadata in JSON sidecar files.

use async_trait::async_trait;
use chrono::Utc;
use std::path::{Path, PathBuf};
use tokio::fs;
use tracing::{debug, info};

use super::traits::{BundleInfo, BundleMetadata, BundleStorage, StorageError, StoredBundle};

/// Filesystem-based bundle storage
pub struct FilesystemStorage {
    /// Base path for storing bundles
    base_path: PathBuf,
}

impl FilesystemStorage {
    /// Create a new filesystem storage
    pub fn new(base_path: &Path) -> Result<Self, StorageError> {
        // Create base directory if it doesn't exist
        if !base_path.exists() {
            std::fs::create_dir_all(base_path)?;
            info!("Created storage directory: {:?}", base_path);
        }

        Ok(Self {
            base_path: base_path.to_path_buf(),
        })
    }

    /// Get the full path for a bundle
    fn bundle_path(&self, key: &str) -> PathBuf {
        self.base_path.join(key)
    }

    /// Get the metadata path for a bundle
    fn metadata_path(&self, key: &str) -> PathBuf {
        self.base_path.join(format!("{}.meta.json", key))
    }

    /// Ensure parent directories exist
    async fn ensure_parent_dirs(&self, path: &Path) -> Result<(), StorageError> {
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent).await?;
            }
        }
        Ok(())
    }
}

#[async_trait]
impl BundleStorage for FilesystemStorage {
    async fn put(
        &self,
        key: &str,
        data: &[u8],
        metadata: BundleMetadata,
    ) -> Result<(), StorageError> {
        let bundle_path = self.bundle_path(key);
        let metadata_path = self.metadata_path(key);

        debug!("Storing bundle at {:?}", bundle_path);

        // Ensure parent directories exist
        self.ensure_parent_dirs(&bundle_path).await?;

        // Write bundle data
        fs::write(&bundle_path, data).await?;

        // Write metadata
        let metadata_json = serde_json::to_string_pretty(&StoredMetadata {
            metadata,
            created_at: Utc::now(),
            size_bytes: data.len() as u64,
        })
        .map_err(|e| StorageError::Serialization(e.to_string()))?;

        fs::write(&metadata_path, metadata_json).await?;

        info!(
            "Stored bundle: {} ({} bytes)",
            key,
            data.len()
        );

        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Option<StoredBundle>, StorageError> {
        let bundle_path = self.bundle_path(key);
        let metadata_path = self.metadata_path(key);

        if !bundle_path.exists() {
            return Ok(None);
        }

        debug!("Reading bundle from {:?}", bundle_path);

        // Read bundle data
        let data = fs::read(&bundle_path).await?;

        // Read metadata
        let metadata_json = fs::read_to_string(&metadata_path).await?;
        let stored: StoredMetadata = serde_json::from_str(&metadata_json)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;

        Ok(Some(StoredBundle {
            data,
            metadata: stored.metadata,
            created_at: stored.created_at,
            storage_key: key.to_string(),
        }))
    }

    async fn delete(&self, key: &str) -> Result<(), StorageError> {
        let bundle_path = self.bundle_path(key);
        let metadata_path = self.metadata_path(key);

        if bundle_path.exists() {
            fs::remove_file(&bundle_path).await?;
        }

        if metadata_path.exists() {
            fs::remove_file(&metadata_path).await?;
        }

        info!("Deleted bundle: {}", key);

        Ok(())
    }

    async fn list(&self, prefix: Option<&str>) -> Result<Vec<BundleInfo>, StorageError> {
        let mut bundles = Vec::new();

        let search_path = match prefix {
            Some(p) => self.base_path.join(p),
            None => self.base_path.clone(),
        };

        if !search_path.exists() {
            return Ok(bundles);
        }

        // Walk the directory tree
        bundles.extend(self.list_recursive(&search_path, prefix).await?);

        Ok(bundles)
    }

    async fn exists(&self, key: &str) -> Result<bool, StorageError> {
        Ok(self.bundle_path(key).exists())
    }

    fn backend_name(&self) -> &'static str {
        "filesystem"
    }
}

impl FilesystemStorage {
    /// Recursively list bundles in a directory
    fn list_recursive<'a>(
        &'a self,
        path: &'a Path,
        prefix: Option<&'a str>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<BundleInfo>, StorageError>> + Send + 'a>> {
        Box::pin(async move {
            let mut bundles = Vec::new();

            let mut entries = fs::read_dir(path).await?;

            while let Some(entry) = entries.next_entry().await? {
                let entry_path = entry.path();

                if entry_path.is_dir() {
                    // Recurse into subdirectories
                    bundles.extend(self.list_recursive(&entry_path, prefix).await?);
                } else if entry_path.extension().map(|e| e == "rpp").unwrap_or(false) {
                    // This is a bundle file
                    let key = entry_path
                        .strip_prefix(&self.base_path)
                        .map_err(|e| StorageError::Operation(e.to_string()))?
                        .to_string_lossy()
                        .to_string();

                    // Check prefix filter
                    if let Some(p) = prefix {
                        if !key.starts_with(p) {
                            continue;
                        }
                    }

                    // Try to read metadata
                    let metadata_path = self.metadata_path(&key);
                    if metadata_path.exists() {
                        if let Ok(metadata_json) = fs::read_to_string(&metadata_path).await {
                            if let Ok(stored) = serde_json::from_str::<StoredMetadata>(&metadata_json) {
                                bundles.push(BundleInfo {
                                    key,
                                    size_bytes: stored.size_bytes,
                                    created_at: stored.created_at,
                                    metadata: stored.metadata,
                                });
                            }
                        }
                    }
                }
            }

            Ok(bundles)
        })
    }
}

/// Internal structure for storing metadata
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct StoredMetadata {
    metadata: BundleMetadata,
    created_at: chrono::DateTime<chrono::Utc>,
    size_bytes: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use uuid::Uuid;

    #[tokio::test]
    async fn test_put_and_get() {
        let temp_dir = TempDir::new().unwrap();
        let storage = FilesystemStorage::new(temp_dir.path()).unwrap();

        let org_id = Uuid::new_v4();
        let bundle_id = Uuid::new_v4();
        let key = format!("{}/{}/1.0.0.rpp", org_id, bundle_id);
        let data = b"test bundle data";
        let metadata = BundleMetadata::new(
            org_id,
            bundle_id,
            "1.0.0".to_string(),
            3,
            "abc123".to_string(),
        );

        storage.put(&key, data, metadata.clone()).await.unwrap();

        let stored = storage.get(&key).await.unwrap().unwrap();
        assert_eq!(stored.data, data);
        assert_eq!(stored.metadata.version, "1.0.0");
        assert_eq!(stored.metadata.policy_count, 3);
    }

    #[tokio::test]
    async fn test_delete() {
        let temp_dir = TempDir::new().unwrap();
        let storage = FilesystemStorage::new(temp_dir.path()).unwrap();

        let key = "test/bundle.rpp";
        let data = b"test data";
        let metadata = BundleMetadata::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            "1.0.0".to_string(),
            1,
            "hash".to_string(),
        );

        storage.put(key, data, metadata).await.unwrap();
        assert!(storage.exists(key).await.unwrap());

        storage.delete(key).await.unwrap();
        assert!(!storage.exists(key).await.unwrap());
    }

    #[tokio::test]
    async fn test_list() {
        let temp_dir = TempDir::new().unwrap();
        let storage = FilesystemStorage::new(temp_dir.path()).unwrap();

        let org_id = Uuid::new_v4();

        // Store multiple bundles
        for i in 0..3 {
            let bundle_id = Uuid::new_v4();
            let key = format!("{}/{}/1.0.0.rpp", org_id, bundle_id);
            let metadata = BundleMetadata::new(
                org_id,
                bundle_id,
                "1.0.0".to_string(),
                i,
                format!("hash{}", i),
            );
            storage.put(&key, b"data", metadata).await.unwrap();
        }

        // List all
        let bundles = storage.list(None).await.unwrap();
        assert_eq!(bundles.len(), 3);

        // List with prefix
        let prefix = org_id.to_string();
        let bundles = storage.list(Some(&prefix)).await.unwrap();
        assert_eq!(bundles.len(), 3);
    }

    #[tokio::test]
    async fn test_not_found() {
        let temp_dir = TempDir::new().unwrap();
        let storage = FilesystemStorage::new(temp_dir.path()).unwrap();

        let result = storage.get("nonexistent.rpp").await.unwrap();
        assert!(result.is_none());

        assert!(!storage.exists("nonexistent.rpp").await.unwrap());
    }
}
