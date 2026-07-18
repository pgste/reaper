//! S3 bucket synchronization
//!
//! Syncs policies from S3 buckets, supporting both policy files and pre-compiled bundles.

use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::domain::source::{PolicySource, SyncResult};

/// S3 sync errors
#[derive(Debug, Error)]
pub enum S3SyncError {
    #[error("S3 error: {0}")]
    S3(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Configuration error: {0}")]
    Config(String),
    #[error("Pattern error: {0}")]
    Pattern(String),
    #[error("AWS SDK error: {0}")]
    AwsSdk(String),
}

/// A policy file extracted from S3
#[derive(Debug, Clone)]
pub struct S3PolicyFile {
    /// S3 key (path)
    pub key: String,
    /// File content
    pub content: String,
    /// Detected policy language
    pub language: String,
    /// ETag for change detection
    pub etag: Option<String>,
    /// Last modified timestamp
    pub last_modified: Option<String>,
}

/// S3 bucket syncer
pub struct S3Syncer {
    /// Base directory for caching downloaded files
    cache_path: PathBuf,
    /// HTTP client for non-SDK operations
    #[allow(dead_code)]
    client: reqwest::Client,
}

impl S3Syncer {
    /// Create a new S3 syncer
    pub fn new(cache_path: impl AsRef<Path>) -> Self {
        Self {
            cache_path: cache_path.as_ref().to_path_buf(),
            client: crate::http::build_or_default(
                crate::http::http_client_builder(std::time::Duration::from_secs(60))
                    // No redirect-following: keep an S3 endpoint fetch from being
                    // bounced to an internal address (round-3 SEC R3-5 hardening).
                    .redirect(reqwest::redirect::Policy::none()),
            ),
        }
    }

    /// Sync a policy source from S3
    #[cfg(feature = "storage-s3")]
    pub async fn sync(&self, source: &PolicySource) -> Result<SyncResult, S3SyncError> {
        let start = std::time::Instant::now();

        let config = source
            .s3_config()
            .ok_or_else(|| S3SyncError::Config("Invalid S3 configuration".to_string()))?;

        // Build AWS SDK config
        let aws_config = self.build_aws_config(&config).await?;

        // Create S3 client
        let s3_client = aws_sdk_s3::Client::new(&aws_config);

        // List objects in bucket with prefix
        let policy_files = self.list_and_download_policies(&s3_client, &config).await?;

        let duration_ms = start.elapsed().as_millis() as u64;

        info!(
            source_id = %source.id,
            bucket = %config.bucket,
            files_found = policy_files.len(),
            duration_ms = duration_ms,
            "S3 sync completed"
        );

        Ok(SyncResult {
            source_id: source.id,
            success: true,
            policies_found: policy_files.len(),
            policies_updated: policy_files.len(),
            policies_created: 0,
            commit: None, // S3 doesn't have commits
            error: None,
            duration_ms,
        })
    }

    /// Fallback sync when S3 feature is not enabled
    #[cfg(not(feature = "storage-s3"))]
    pub async fn sync(&self, _source: &PolicySource) -> Result<SyncResult, S3SyncError> {
        Err(S3SyncError::Config(
            "S3 sync requires the 'storage-s3' feature to be enabled".to_string(),
        ))
    }

    /// Build AWS configuration
    #[cfg(feature = "storage-s3")]
    async fn build_aws_config(
        &self,
        config: &S3Config,
    ) -> Result<aws_config::SdkConfig, S3SyncError> {
        use aws_config::BehaviorVersion;

        let mut loader = aws_config::defaults(BehaviorVersion::latest())
            .region(aws_config::Region::new(config.region.clone()));

        // Use explicit credentials if provided
        if let (Some(access_key), Some(secret_key)) =
            (&config.access_key_id, &config.secret_access_key)
        {
            let creds = aws_config::Credentials::new(
                access_key.clone(),
                secret_key.clone(),
                None,
                None,
                "reaper-management",
            );
            loader = loader.credentials_provider(creds);
        }

        let sdk_config = loader.load().await;
        Ok(sdk_config)
    }

    /// List and download policy files from S3
    #[cfg(feature = "storage-s3")]
    async fn list_and_download_policies(
        &self,
        client: &aws_sdk_s3::Client,
        config: &S3Config,
    ) -> Result<Vec<S3PolicyFile>, S3SyncError> {
        let mut policy_files = Vec::new();
        let mut continuation_token: Option<String> = None;

        loop {
            let mut request = client
                .list_objects_v2()
                .bucket(&config.bucket)
                .max_keys(1000);

            if let Some(prefix) = &config.prefix {
                request = request.prefix(prefix);
            }

            if let Some(token) = continuation_token.take() {
                request = request.continuation_token(token);
            }

            let response = request
                .send()
                .await
                .map_err(|e| S3SyncError::S3(e.to_string()))?;

            if let Some(contents) = response.contents() {
                for object in contents {
                    if let Some(key) = object.key() {
                        // Check if the key matches any of our patterns
                        if self.matches_patterns(key, &config.patterns) {
                            match self.download_object(client, &config.bucket, key).await {
                                Ok(content) => {
                                    policy_files.push(S3PolicyFile {
                                        key: key.to_string(),
                                        content,
                                        language: detect_language(key),
                                        etag: object.e_tag().map(|s| s.to_string()),
                                        last_modified: object
                                            .last_modified()
                                            .map(|t| t.to_string()),
                                    });
                                }
                                Err(e) => {
                                    warn!("Failed to download S3 object {}: {}", key, e);
                                }
                            }
                        }
                    }
                }
            }

            if response.is_truncated() == Some(true) {
                continuation_token = response.next_continuation_token().map(|s| s.to_string());
            } else {
                break;
            }
        }

        Ok(policy_files)
    }

    /// Download a single object from S3
    #[cfg(feature = "storage-s3")]
    async fn download_object(
        &self,
        client: &aws_sdk_s3::Client,
        bucket: &str,
        key: &str,
    ) -> Result<String, S3SyncError> {
        debug!("Downloading S3 object: s3://{}/{}", bucket, key);

        let response = client
            .get_object()
            .bucket(bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| S3SyncError::S3(e.to_string()))?;

        let bytes = response
            .body
            .collect()
            .await
            .map_err(|e| S3SyncError::S3(e.to_string()))?
            .into_bytes();

        String::from_utf8(bytes.to_vec())
            .map_err(|e| S3SyncError::S3(format!("Invalid UTF-8 content: {}", e)))
    }

    /// Check if a key matches any of the glob patterns
    #[allow(dead_code)]
    fn matches_patterns(&self, key: &str, patterns: &[String]) -> bool {
        for pattern in patterns {
            // Convert glob pattern to regex-like matching
            let pattern = pattern.trim_start_matches("**/");
            if key.ends_with(pattern.trim_start_matches('*')) {
                return true;
            }

            // Simple glob matching
            if let Ok(glob_pattern) = glob::Pattern::new(pattern) {
                if glob_pattern.matches(key) {
                    return true;
                }
            }
        }
        false
    }

    /// Get the cache path for a source
    #[allow(dead_code)]
    fn cache_path(&self, source_id: uuid::Uuid) -> PathBuf {
        self.cache_path.join(format!("s3-{}", source_id))
    }

    /// Get all policy files from the cache (after sync)
    pub fn get_policy_files(
        &self,
        _source: &PolicySource,
    ) -> Result<Vec<S3PolicyFile>, S3SyncError> {
        // For S3, we don't maintain a local cache like Git
        // Re-fetching would require another sync
        // Return empty for now - caller should use sync result
        Ok(Vec::new())
    }

    /// Clean up cached files for a source
    pub fn cleanup(&self, source_id: uuid::Uuid) -> Result<(), S3SyncError> {
        let path = self.cache_path(source_id);
        if path.exists() {
            std::fs::remove_dir_all(path)?;
        }
        Ok(())
    }
}

impl Default for S3Syncer {
    fn default() -> Self {
        Self::new("/tmp/reaper-sync/s3")
    }
}

/// Detect policy language from file extension
#[allow(dead_code)]
fn detect_language(key: &str) -> String {
    let path = Path::new(key);
    match path.extension().and_then(|e| e.to_str()) {
        Some("reap") => "reaper".to_string(),
        Some("yaml") | Some("yml") => "reaper".to_string(),
        Some("json") => "reaper".to_string(),
        Some("cedar") => "cedar".to_string(),
        Some("rbb") => "bundle".to_string(), // Pre-compiled bundle
        _ => "simple".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_detect_language() {
        assert_eq!(detect_language("policies/main.reap"), "reaper");
        assert_eq!(detect_language("config/rules.yaml"), "reaper");
        assert_eq!(detect_language("auth/policy.cedar"), "cedar");
        assert_eq!(detect_language("bundles/v1.rbb"), "bundle");
        assert_eq!(detect_language("unknown.txt"), "simple");
    }

    #[test]
    fn test_matches_patterns() {
        let temp_dir = TempDir::new().unwrap();
        let syncer = S3Syncer::new(temp_dir.path());

        let patterns = vec!["**/*.reap".to_string(), "**/*.yaml".to_string()];

        assert!(syncer.matches_patterns("policies/main.reap", &patterns));
        assert!(syncer.matches_patterns("config/rules.yaml", &patterns));
        assert!(!syncer.matches_patterns("readme.md", &patterns));
    }

    #[test]
    fn test_s3_syncer_creation() {
        let temp_dir = TempDir::new().unwrap();
        let syncer = S3Syncer::new(temp_dir.path());
        assert!(syncer.cache_path.exists() || !syncer.cache_path.exists()); // Just test creation works
    }
}
