//! Policy Cache - Disk Persistence for Policies
//!
//! Persists deployed policies to disk so they survive agent restarts.
//! Uses JSON format for human readability and debugging.

use policy_engine::EnhancedPolicy;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use thiserror::Error;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// Policy cache errors
#[derive(Debug, Error)]
pub enum PolicyCacheError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("Cache directory not found: {0}")]
    DirectoryNotFound(PathBuf),
}

/// Cached policy representation (serializable subset of EnhancedPolicy)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedPolicy {
    pub id: Uuid,
    pub version: u64,
    pub name: String,
    pub description: String,
    pub language: String,
    pub content: String,
    pub metadata: std::collections::HashMap<String, String>,
    pub priority: u32,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub source_metadata: Option<CachedSourceMetadata>,
}

/// Cached source metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedSourceMetadata {
    pub source_type: String,
    pub source_details: serde_json::Value,
    pub deployed_at: chrono::DateTime<chrono::Utc>,
    pub deployed_by: Option<String>,
    pub source_version: Option<String>,
    pub checksum: Option<String>,
}

impl From<&EnhancedPolicy> for CachedPolicy {
    fn from(policy: &EnhancedPolicy) -> Self {
        let language = match &policy.language {
            policy_engine::PolicyLanguage::Simple => "simple",
            policy_engine::PolicyLanguage::Cedar => "cedar",
            policy_engine::PolicyLanguage::Custom => "custom",
        };

        let source_metadata = policy.source_metadata.as_ref().map(|sm| {
            let (source_type, source_details) = match &sm.source {
                policy_engine::PolicySource::File { path } => {
                    ("file", serde_json::json!({ "path": path }))
                }
                policy_engine::PolicySource::Api { client_id } => {
                    ("api", serde_json::json!({ "client_id": client_id }))
                }
                policy_engine::PolicySource::SyncClient {
                    server_url,
                    server_version,
                    team,
                } => (
                    "sync_client",
                    serde_json::json!({
                        "server_url": server_url,
                        "server_version": server_version,
                        "team": team
                    }),
                ),
                policy_engine::PolicySource::Default => ("default", serde_json::json!({})),
            };

            CachedSourceMetadata {
                source_type: source_type.to_string(),
                source_details,
                deployed_at: sm.deployed_at,
                deployed_by: sm.deployed_by.clone(),
                source_version: sm.source_version.clone(),
                checksum: sm.checksum.clone(),
            }
        });

        Self {
            id: policy.id,
            version: policy.version,
            name: policy.name.clone(),
            description: policy.description.clone(),
            language: language.to_string(),
            content: policy.content.clone(),
            metadata: policy.metadata.clone(),
            priority: policy.priority,
            created_at: policy.created_at,
            updated_at: policy.updated_at,
            source_metadata,
        }
    }
}

impl CachedPolicy {
    /// Convert cached policy back to EnhancedPolicy
    ///
    /// Note: The evaluator will need to be rebuilt separately
    pub fn to_enhanced_policy(&self) -> Result<EnhancedPolicy, PolicyCacheError> {
        let language = match self.language.as_str() {
            "simple" => policy_engine::PolicyLanguage::Simple,
            "cedar" => policy_engine::PolicyLanguage::Cedar,
            "custom" => policy_engine::PolicyLanguage::Custom,
            _ => policy_engine::PolicyLanguage::Simple,
        };

        let source_metadata = self.source_metadata.as_ref().map(|sm| {
            let source = match sm.source_type.as_str() {
                "file" => {
                    let path = sm.source_details["path"]
                        .as_str()
                        .unwrap_or("")
                        .to_string();
                    policy_engine::PolicySource::File { path }
                }
                "api" => {
                    let client_id = sm.source_details["client_id"]
                        .as_str()
                        .map(|s| s.to_string());
                    policy_engine::PolicySource::Api { client_id }
                }
                "sync_client" => {
                    let server_url = sm.source_details["server_url"]
                        .as_str()
                        .unwrap_or("")
                        .to_string();
                    let server_version = sm.source_details["server_version"]
                        .as_str()
                        .unwrap_or("")
                        .to_string();
                    let team = sm.source_details["team"].as_str().map(|s| s.to_string());
                    policy_engine::PolicySource::SyncClient {
                        server_url,
                        server_version,
                        team,
                    }
                }
                _ => policy_engine::PolicySource::Default,
            };

            policy_engine::PolicySourceMetadata {
                source,
                deployed_at: sm.deployed_at,
                deployed_by: sm.deployed_by.clone(),
                source_version: sm.source_version.clone(),
                checksum: sm.checksum.clone(),
            }
        });

        Ok(EnhancedPolicy {
            id: self.id,
            version: self.version,
            name: self.name.clone(),
            description: self.description.clone(),
            language,
            content: self.content.clone(),
            rules: vec![], // Will be rebuilt from content
            metadata: self.metadata.clone(),
            priority: self.priority,
            created_at: self.created_at,
            updated_at: self.updated_at,
            evaluator: None, // Will be rebuilt
            source_metadata,
        })
    }
}

/// Policy cache for disk persistence
pub struct PolicyCache {
    cache_dir: PathBuf,
}

impl PolicyCache {
    /// Create a new policy cache
    ///
    /// Creates the cache directory if it doesn't exist
    pub fn new(cache_dir: PathBuf) -> Result<Self, PolicyCacheError> {
        // Create directory if it doesn't exist
        if !cache_dir.exists() {
            std::fs::create_dir_all(&cache_dir)?;
            info!("Created policy cache directory: {:?}", cache_dir);
        }
        Ok(Self { cache_dir })
    }

    /// Save a policy to the cache
    pub async fn save_policy(&self, policy: &EnhancedPolicy) -> Result<(), PolicyCacheError> {
        let cached = CachedPolicy::from(policy);
        let filename = format!("{}.json", policy.id);
        let path = self.cache_dir.join(filename);

        let json = serde_json::to_string_pretty(&cached)?;
        tokio::fs::write(&path, json).await?;

        debug!("Cached policy {} to {:?}", policy.id, path);
        Ok(())
    }

    /// Load all cached policies
    pub async fn load_policies(&self) -> Result<Vec<EnhancedPolicy>, PolicyCacheError> {
        let mut policies = Vec::new();

        if !self.cache_dir.exists() {
            return Ok(policies);
        }

        let mut entries = tokio::fs::read_dir(&self.cache_dir).await?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();

            // Only process .json files
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }

            match self.load_single_policy(&path).await {
                Ok(policy) => {
                    info!("Loaded cached policy: {} ({})", policy.name, policy.id);
                    policies.push(policy);
                }
                Err(e) => {
                    warn!("Failed to load cached policy from {:?}: {}", path, e);
                }
            }
        }

        info!("Loaded {} policies from cache", policies.len());
        Ok(policies)
    }

    /// Load a single policy from cache
    async fn load_single_policy(&self, path: &PathBuf) -> Result<EnhancedPolicy, PolicyCacheError> {
        let content = tokio::fs::read_to_string(path).await?;
        let cached: CachedPolicy = serde_json::from_str(&content)?;
        cached.to_enhanced_policy()
    }

    /// Delete a policy from the cache
    pub async fn delete_policy(&self, policy_id: &Uuid) -> Result<(), PolicyCacheError> {
        let filename = format!("{}.json", policy_id);
        let path = self.cache_dir.join(filename);

        if path.exists() {
            tokio::fs::remove_file(&path).await?;
            debug!("Removed cached policy: {}", policy_id);
        }

        Ok(())
    }

    /// Clear all cached policies
    pub async fn clear(&self) -> Result<usize, PolicyCacheError> {
        let mut count = 0;

        if !self.cache_dir.exists() {
            return Ok(0);
        }

        let mut entries = tokio::fs::read_dir(&self.cache_dir).await?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                if let Err(e) = tokio::fs::remove_file(&path).await {
                    error!("Failed to remove cached policy {:?}: {}", path, e);
                } else {
                    count += 1;
                }
            }
        }

        info!("Cleared {} policies from cache", count);
        Ok(count)
    }

    /// Get cache statistics
    pub async fn stats(&self) -> Result<CacheStats, PolicyCacheError> {
        let mut policy_count = 0;
        let mut total_size = 0u64;

        if self.cache_dir.exists() {
            let mut entries = tokio::fs::read_dir(&self.cache_dir).await?;

            while let Some(entry) = entries.next_entry().await? {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) == Some("json") {
                    policy_count += 1;
                    if let Ok(metadata) = tokio::fs::metadata(&path).await {
                        total_size += metadata.len();
                    }
                }
            }
        }

        Ok(CacheStats {
            policy_count,
            total_size_bytes: total_size,
            cache_dir: self.cache_dir.clone(),
        })
    }
}

/// Cache statistics
#[derive(Debug)]
pub struct CacheStats {
    pub policy_count: usize,
    pub total_size_bytes: u64,
    pub cache_dir: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_policy_cache_save_load() {
        let temp_dir = TempDir::new().unwrap();
        let cache = PolicyCache::new(temp_dir.path().to_path_buf()).unwrap();

        // Create a test policy
        let policy = EnhancedPolicy::new(
            "test-policy".to_string(),
            "Test policy".to_string(),
            vec![],
        );

        // Save to cache
        cache.save_policy(&policy).await.unwrap();

        // Load from cache
        let policies = cache.load_policies().await.unwrap();
        assert_eq!(policies.len(), 1);
        assert_eq!(policies[0].name, "test-policy");
    }

    #[tokio::test]
    async fn test_policy_cache_delete() {
        let temp_dir = TempDir::new().unwrap();
        let cache = PolicyCache::new(temp_dir.path().to_path_buf()).unwrap();

        let policy = EnhancedPolicy::new(
            "test-policy".to_string(),
            "Test policy".to_string(),
            vec![],
        );

        cache.save_policy(&policy).await.unwrap();
        cache.delete_policy(&policy.id).await.unwrap();

        let policies = cache.load_policies().await.unwrap();
        assert!(policies.is_empty());
    }

    #[tokio::test]
    async fn test_policy_cache_stats() {
        let temp_dir = TempDir::new().unwrap();
        let cache = PolicyCache::new(temp_dir.path().to_path_buf()).unwrap();

        let policy = EnhancedPolicy::new(
            "test-policy".to_string(),
            "Test policy".to_string(),
            vec![],
        );

        cache.save_policy(&policy).await.unwrap();

        let stats = cache.stats().await.unwrap();
        assert_eq!(stats.policy_count, 1);
        assert!(stats.total_size_bytes > 0);
    }
}
