//! Storage traits and types
//!
//! Defines the BundleStorage trait that all storage backends must implement.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

/// Storage errors
#[derive(Debug, Error)]
pub enum StorageError {
    #[error("Storage configuration error: {0}")]
    Config(String),

    #[error("Storage IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Bundle not found: {0}")]
    NotFound(String),

    #[error("Storage operation failed: {0}")]
    Operation(String),

    #[error("Serialization error: {0}")]
    Serialization(String),
}

/// Metadata associated with a stored bundle
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleMetadata {
    /// Organization ID
    pub org_id: Uuid,
    /// Bundle ID
    pub bundle_id: Uuid,
    /// Bundle version string
    pub version: String,
    /// Number of policies in the bundle
    pub policy_count: usize,
    /// SHA-256 checksum of the bundle data
    pub checksum: String,
    /// Content type (application/octet-stream for .rpp)
    #[serde(default = "default_content_type")]
    pub content_type: String,
    /// Custom tags for filtering/searching
    #[serde(default)]
    pub tags: Vec<String>,
}

fn default_content_type() -> String {
    "application/octet-stream".to_string()
}

/// A stored bundle with its data and metadata
#[derive(Debug, Clone)]
pub struct StoredBundle {
    /// The bundle binary data
    pub data: Vec<u8>,
    /// Bundle metadata
    pub metadata: BundleMetadata,
    /// When the bundle was stored
    pub created_at: DateTime<Utc>,
    /// Storage-specific key/path
    pub storage_key: String,
}

/// Summary information about a stored bundle (without data)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleInfo {
    /// Storage key
    pub key: String,
    /// Bundle size in bytes
    pub size_bytes: u64,
    /// When the bundle was stored
    pub created_at: DateTime<Utc>,
    /// Bundle metadata
    pub metadata: BundleMetadata,
}

/// Trait for bundle storage backends
#[async_trait]
pub trait BundleStorage: Send + Sync {
    /// Store a bundle with its metadata
    async fn put(
        &self,
        key: &str,
        data: &[u8],
        metadata: BundleMetadata,
    ) -> Result<(), StorageError>;

    /// Retrieve a bundle by its key
    async fn get(&self, key: &str) -> Result<Option<StoredBundle>, StorageError>;

    /// Delete a bundle by its key
    async fn delete(&self, key: &str) -> Result<(), StorageError>;

    /// List bundles with optional prefix filter
    async fn list(&self, prefix: Option<&str>) -> Result<Vec<BundleInfo>, StorageError>;

    /// Check if a bundle exists
    async fn exists(&self, key: &str) -> Result<bool, StorageError>;

    /// Get the storage backend name
    fn backend_name(&self) -> &'static str;
}

impl BundleMetadata {
    /// Create new metadata for a bundle
    pub fn new(
        org_id: Uuid,
        bundle_id: Uuid,
        version: String,
        policy_count: usize,
        checksum: String,
    ) -> Self {
        Self {
            org_id,
            bundle_id,
            version,
            policy_count,
            checksum,
            content_type: default_content_type(),
            tags: Vec::new(),
        }
    }

    /// Add a tag to the metadata
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.push(tag.into());
        self
    }

    /// Set content type
    pub fn with_content_type(mut self, content_type: impl Into<String>) -> Self {
        self.content_type = content_type.into();
        self
    }
}

/// Generate a storage key for a bundle
pub fn generate_storage_key(org_id: Uuid, bundle_id: Uuid, version: &str) -> String {
    format!("{}/{}/{}.rpp", org_id, bundle_id, version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_storage_key() {
        let org_id = Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap();
        let bundle_id = Uuid::parse_str("22222222-2222-2222-2222-222222222222").unwrap();
        let key = generate_storage_key(org_id, bundle_id, "1.0.0");
        assert_eq!(
            key,
            "11111111-1111-1111-1111-111111111111/22222222-2222-2222-2222-222222222222/1.0.0.rpp"
        );
    }

    #[test]
    fn test_metadata_builder() {
        let org_id = Uuid::new_v4();
        let bundle_id = Uuid::new_v4();

        let metadata = BundleMetadata::new(
            org_id,
            bundle_id,
            "1.0.0".to_string(),
            5,
            "abc123".to_string(),
        )
        .with_tag("production")
        .with_content_type("application/x-reaper-bundle");

        assert_eq!(metadata.policy_count, 5);
        assert_eq!(metadata.tags, vec!["production"]);
        assert_eq!(metadata.content_type, "application/x-reaper-bundle");
    }
}
