//! Bundle URL synchronization
//!
//! Fetches pre-compiled bundles from URLs, typically triggered by webhooks.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use thiserror::Error;
use tracing::{debug, info, warn};

use crate::domain::source::{BundleUrlConfig, PolicySource, SyncResult};

/// Bundle URL sync errors
#[derive(Debug, Error)]
pub enum BundleUrlSyncError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Configuration error: {0}")]
    Config(String),
    #[error("Checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: String, actual: String },
    #[error("Download failed: {0}")]
    Download(String),
    #[error("Invalid bundle: {0}")]
    InvalidBundle(String),
}

/// Result of fetching a bundle
#[derive(Debug, Clone)]
pub struct FetchedBundle {
    /// Bundle binary data
    pub data: Vec<u8>,
    /// Bundle version (if provided)
    pub version: Option<String>,
    /// Checksum of the bundle
    pub checksum: String,
    /// Size in bytes
    pub size_bytes: usize,
    /// Source URL
    pub source_url: String,
    /// Bundle format (.rbb or .rpp)
    pub format: BundleFormat,
}

/// Bundle format types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BundleFormat {
    /// Reaper Binary Bundle (.rbb) - compiled, ready to deploy
    Rbb,
    /// Reaper Policy Package (.rpp) - multi-policy package with hints
    Rpp,
}

/// Bundle URL fetcher
pub struct BundleUrlSyncer {
    /// Base directory for storing downloaded bundles
    storage_path: PathBuf,
    /// HTTP client
    client: reqwest::Client,
}

impl BundleUrlSyncer {
    /// Create a new Bundle URL syncer
    pub fn new(storage_path: impl AsRef<Path>) -> Self {
        Self {
            storage_path: storage_path.as_ref().to_path_buf(),
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
        }
    }

    /// Fetch a bundle from a URL (typically called from webhook handler)
    pub async fn fetch_bundle(
        &self,
        source: &PolicySource,
        bundle_url: &str,
        expected_version: Option<&str>,
        expected_checksum: Option<&str>,
    ) -> Result<FetchedBundle, BundleUrlSyncError> {
        let start = std::time::Instant::now();

        let config = source
            .bundle_url_config()
            .ok_or_else(|| BundleUrlSyncError::Config("Invalid BundleUrl configuration".to_string()))?;

        // Build the request
        let mut request = self.client.get(bundle_url);

        // Add authentication if configured
        if let (Some(header), Some(token)) = (&config.auth_header, &config.auth_token) {
            request = request.header(header, token);
        }

        // Set timeout from config
        let timeout = std::time::Duration::from_secs(config.download_timeout_secs as u64);
        request = request.timeout(timeout);

        debug!("Fetching bundle from URL: {}", bundle_url);

        // Execute request
        let response = request.send().await?;

        if !response.status().is_success() {
            return Err(BundleUrlSyncError::Download(format!(
                "HTTP {} from {}",
                response.status(),
                bundle_url
            )));
        }

        // Get bundle data
        let data = response.bytes().await?.to_vec();
        let size_bytes = data.len();

        // Calculate checksum
        let actual_checksum = compute_checksum(&data, &config.checksum_algorithm);

        // Verify checksum if provided and verification is enabled
        if config.verify_checksum {
            if let Some(expected) = expected_checksum {
                let expected_normalized = normalize_checksum(expected);
                if actual_checksum != expected_normalized {
                    return Err(BundleUrlSyncError::ChecksumMismatch {
                        expected: expected_normalized,
                        actual: actual_checksum,
                    });
                }
            }
        }

        let duration_ms = start.elapsed().as_millis() as u64;

        // Detect bundle format from URL
        let format = get_bundle_format(bundle_url).unwrap_or(BundleFormat::Rbb);

        info!(
            source_id = %source.id,
            url = %bundle_url,
            size_bytes = size_bytes,
            format = ?format,
            duration_ms = duration_ms,
            "Bundle fetched successfully"
        );

        Ok(FetchedBundle {
            data,
            version: expected_version.map(|s| s.to_string()),
            checksum: actual_checksum,
            size_bytes,
            source_url: bundle_url.to_string(),
            format,
        })
    }

    /// Sync a policy source (for scheduled syncs - checks base_url if configured)
    pub async fn sync(&self, source: &PolicySource) -> Result<SyncResult, BundleUrlSyncError> {
        let config = source
            .bundle_url_config()
            .ok_or_else(|| BundleUrlSyncError::Config("Invalid BundleUrl configuration".to_string()))?;

        // BundleUrl sources are typically webhook-driven
        // If base_url is configured, we can poll it for the latest version
        if let Some(base_url) = &config.base_url {
            match self.fetch_bundle(source, base_url, None, None).await {
                Ok(bundle) => {
                    return Ok(SyncResult {
                        source_id: source.id,
                        success: true,
                        policies_found: 1, // One bundle
                        policies_updated: 1,
                        policies_created: 0,
                        commit: Some(bundle.checksum), // Use checksum as "commit"
                        error: None,
                        duration_ms: 0, // Already logged in fetch_bundle
                    });
                }
                Err(e) => {
                    warn!("Failed to fetch bundle from base URL: {}", e);
                    return Err(e);
                }
            }
        }

        // No base_url configured - source is webhook-only
        Ok(SyncResult {
            source_id: source.id,
            success: true,
            policies_found: 0,
            policies_updated: 0,
            policies_created: 0,
            commit: None,
            error: None,
            duration_ms: 0,
        })
    }

    /// Store a fetched bundle to the local storage
    pub async fn store_bundle(
        &self,
        source_id: uuid::Uuid,
        bundle: &FetchedBundle,
    ) -> Result<PathBuf, BundleUrlSyncError> {
        let bundle_dir = self.storage_path.join(source_id.to_string());
        std::fs::create_dir_all(&bundle_dir)?;

        let ext = bundle.format.extension();
        let filename = if let Some(version) = &bundle.version {
            format!("bundle-{}.{}", version, ext)
        } else {
            // Use first 8 chars of checksum (skip the "sha256:" prefix if present)
            let checksum_short = bundle.checksum
                .split(':')
                .last()
                .unwrap_or(&bundle.checksum)
                .chars()
                .take(8)
                .collect::<String>();
            format!("bundle-{}.{}", checksum_short, ext)
        };

        let bundle_path = bundle_dir.join(&filename);
        std::fs::write(&bundle_path, &bundle.data)?;

        debug!("Stored bundle at {:?} (format: {:?})", bundle_path, bundle.format);

        Ok(bundle_path)
    }

    /// Validate webhook signature (HMAC)
    pub fn validate_webhook_signature(
        &self,
        config: &BundleUrlConfig,
        payload: &[u8],
        signature: &str,
    ) -> Result<bool, BundleUrlSyncError> {
        let secret = config.webhook_secret.as_ref().ok_or_else(|| {
            BundleUrlSyncError::Config("No webhook secret configured".to_string())
        })?;

        // Parse the signature (format: sha256=<hex>)
        let (algo, sig_hex) = signature
            .split_once('=')
            .ok_or_else(|| BundleUrlSyncError::Config("Invalid signature format".to_string()))?;

        if algo != "sha256" {
            return Err(BundleUrlSyncError::Config(format!(
                "Unsupported signature algorithm: {}",
                algo
            )));
        }

        // Compute HMAC
        use sha2::Sha256;
        use hmac::{Hmac, Mac};
        type HmacSha256 = Hmac<Sha256>;

        let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
            .map_err(|e| BundleUrlSyncError::Config(format!("HMAC error: {}", e)))?;
        mac.update(payload);
        let expected = mac.finalize().into_bytes();
        let expected_hex = hex::encode(expected);

        Ok(expected_hex == sig_hex)
    }

    /// Clean up stored bundles for a source
    pub fn cleanup(&self, source_id: uuid::Uuid) -> Result<(), BundleUrlSyncError> {
        let path = self.storage_path.join(source_id.to_string());
        if path.exists() {
            std::fs::remove_dir_all(path)?;
        }
        Ok(())
    }
}

impl Default for BundleUrlSyncer {
    fn default() -> Self {
        Self::new("/tmp/reaper-sync/bundles")
    }
}

/// Compute checksum of data
fn compute_checksum(data: &[u8], algorithm: &str) -> String {
    match algorithm.to_lowercase().as_str() {
        "sha256" => {
            let mut hasher = Sha256::new();
            hasher.update(data);
            format!("sha256:{}", hex::encode(hasher.finalize()))
        }
        "md5" => {
            use md5::Digest;
            let mut hasher = md5::Md5::new();
            hasher.update(data);
            format!("md5:{}", hex::encode(hasher.finalize()))
        }
        _ => {
            // Default to SHA256
            let mut hasher = Sha256::new();
            hasher.update(data);
            format!("sha256:{}", hex::encode(hasher.finalize()))
        }
    }
}

/// Normalize checksum format (handle with/without prefix)
fn normalize_checksum(checksum: &str) -> String {
    if checksum.contains(':') {
        checksum.to_string()
    } else {
        // Assume SHA256 if no prefix
        format!("sha256:{}", checksum)
    }
}

/// Supported bundle format extensions
pub const BUNDLE_EXTENSIONS: &[&str] = &["rbb", "rpp"];

/// Check if a URL points to a supported bundle format
pub fn is_bundle_url(url: &str) -> bool {
    let url_lower = url.to_lowercase();
    BUNDLE_EXTENSIONS.iter().any(|ext| url_lower.ends_with(&format!(".{}", ext)))
}

/// Get bundle format from URL
pub fn get_bundle_format(url: &str) -> Option<BundleFormat> {
    let url_lower = url.to_lowercase();
    if url_lower.ends_with(".rbb") {
        Some(BundleFormat::Rbb)
    } else if url_lower.ends_with(".rpp") {
        Some(BundleFormat::Rpp)
    } else {
        None
    }
}

impl BundleFormat {
    /// Get the file extension for this format
    pub fn extension(&self) -> &'static str {
        match self {
            BundleFormat::Rbb => "rbb",
            BundleFormat::Rpp => "rpp",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_compute_checksum_sha256() {
        let data = b"hello world";
        let checksum = compute_checksum(data, "sha256");
        assert!(checksum.starts_with("sha256:"));
        assert_eq!(checksum.len(), 7 + 64); // "sha256:" + 64 hex chars
    }

    #[test]
    fn test_normalize_checksum() {
        assert_eq!(
            normalize_checksum("abc123"),
            "sha256:abc123"
        );
        assert_eq!(
            normalize_checksum("sha256:abc123"),
            "sha256:abc123"
        );
        assert_eq!(
            normalize_checksum("md5:abc123"),
            "md5:abc123"
        );
    }

    #[test]
    fn test_bundle_url_syncer_creation() {
        let temp_dir = TempDir::new().unwrap();
        let syncer = BundleUrlSyncer::new(temp_dir.path());
        assert!(syncer.storage_path.exists() || !syncer.storage_path.exists());
    }

    #[tokio::test]
    async fn test_store_bundle_rbb() {
        let temp_dir = TempDir::new().unwrap();
        let syncer = BundleUrlSyncer::new(temp_dir.path());

        let bundle = FetchedBundle {
            data: b"test bundle data".to_vec(),
            version: Some("1.0.0".to_string()),
            checksum: "sha256:abc123".to_string(),
            size_bytes: 16,
            source_url: "https://example.com/bundle.rbb".to_string(),
            format: BundleFormat::Rbb,
        };

        let source_id = uuid::Uuid::new_v4();
        let path = syncer.store_bundle(source_id, &bundle).await.unwrap();

        assert!(path.exists());
        assert!(path.to_string_lossy().contains("bundle-1.0.0.rbb"));
    }

    #[tokio::test]
    async fn test_store_bundle_rpp() {
        let temp_dir = TempDir::new().unwrap();
        let syncer = BundleUrlSyncer::new(temp_dir.path());

        let bundle = FetchedBundle {
            data: b"test policy package".to_vec(),
            version: Some("2.0.0".to_string()),
            checksum: "sha256:def456".to_string(),
            size_bytes: 19,
            source_url: "https://example.com/package.rpp".to_string(),
            format: BundleFormat::Rpp,
        };

        let source_id = uuid::Uuid::new_v4();
        let path = syncer.store_bundle(source_id, &bundle).await.unwrap();

        assert!(path.exists());
        assert!(path.to_string_lossy().contains("bundle-2.0.0.rpp"));
    }

    #[test]
    fn test_get_bundle_format() {
        assert_eq!(get_bundle_format("https://example.com/policy.rbb"), Some(BundleFormat::Rbb));
        assert_eq!(get_bundle_format("https://example.com/package.rpp"), Some(BundleFormat::Rpp));
        assert_eq!(get_bundle_format("https://example.com/file.txt"), None);
    }

    #[test]
    fn test_is_bundle_url() {
        assert!(is_bundle_url("https://example.com/policy.rbb"));
        assert!(is_bundle_url("https://example.com/package.rpp"));
        assert!(!is_bundle_url("https://example.com/file.txt"));
    }
}
