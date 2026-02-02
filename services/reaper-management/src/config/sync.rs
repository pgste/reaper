//! Sync configuration for policy sources

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use super::error::ConfigError;

/// Sync configuration for policy sources
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SyncConfig {
    /// Base path for Git repositories
    #[serde(default = "default_git_base_path")]
    pub git_base_path: PathBuf,
    /// Base path for S3 cache
    #[serde(default = "default_s3_cache_path")]
    pub s3_cache_path: PathBuf,
    /// Base path for bundle URL storage
    #[serde(default = "default_bundle_storage_path")]
    pub bundle_storage_path: PathBuf,
    /// Interval to check for due syncs
    #[serde(default = "default_sync_check_interval")]
    pub check_interval_secs: u64,
    /// Maximum concurrent sync operations
    #[serde(default = "default_max_concurrent_syncs")]
    pub max_concurrent: usize,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            git_base_path: default_git_base_path(),
            s3_cache_path: default_s3_cache_path(),
            bundle_storage_path: default_bundle_storage_path(),
            check_interval_secs: default_sync_check_interval(),
            max_concurrent: default_max_concurrent_syncs(),
        }
    }
}

impl SyncConfig {
    /// Validate sync configuration
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.check_interval_secs == 0 {
            return Err(ConfigError::InvalidTimeout(
                "check_interval_secs must be positive".to_string(),
            ));
        }

        if self.max_concurrent == 0 {
            return Err(ConfigError::InvalidRateLimit(
                "max_concurrent must be positive".to_string(),
            ));
        }

        Ok(())
    }
}

pub(super) fn default_git_base_path() -> PathBuf {
    PathBuf::from("/var/lib/reaper/git")
}

fn default_s3_cache_path() -> PathBuf {
    PathBuf::from("/var/lib/reaper/sync/s3")
}

fn default_bundle_storage_path() -> PathBuf {
    PathBuf::from("/var/lib/reaper/sync/bundles")
}

fn default_sync_check_interval() -> u64 {
    60
}

fn default_max_concurrent_syncs() -> usize {
    5
}
