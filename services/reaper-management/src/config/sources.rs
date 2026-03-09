//! Policy sources configuration

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use super::sync::default_git_base_path;

/// Policy sources configuration
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct SourcesConfig {
    #[serde(default)]
    pub git: GitSourceConfig,
    #[serde(default)]
    pub api: ApiSourceConfig,
    #[serde(default)]
    pub s3: S3SourceConfig,
    #[serde(default)]
    pub bundle_url: BundleUrlSourceConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GitSourceConfig {
    #[serde(default = "default_git_base_path")]
    pub work_dir: PathBuf,
    #[serde(default = "default_git_poll_interval")]
    pub default_poll_interval_seconds: u64,
}

impl Default for GitSourceConfig {
    fn default() -> Self {
        Self {
            work_dir: default_git_base_path(),
            default_poll_interval_seconds: default_git_poll_interval(),
        }
    }
}

fn default_git_poll_interval() -> u64 {
    60
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ApiSourceConfig {
    #[serde(default = "default_api_poll_interval")]
    pub default_poll_interval_seconds: u64,
    #[serde(default = "default_api_timeout")]
    pub default_timeout_seconds: u64,
}

impl Default for ApiSourceConfig {
    fn default() -> Self {
        Self {
            default_poll_interval_seconds: default_api_poll_interval(),
            default_timeout_seconds: default_api_timeout(),
        }
    }
}

fn default_api_poll_interval() -> u64 {
    300
}

fn default_api_timeout() -> u64 {
    30
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct S3SourceConfig {
    #[serde(default = "default_s3_poll_interval")]
    pub default_poll_interval_seconds: u64,
    #[serde(default)]
    pub default_region: Option<String>,
}

impl Default for S3SourceConfig {
    fn default() -> Self {
        Self {
            default_poll_interval_seconds: default_s3_poll_interval(),
            default_region: None,
        }
    }
}

fn default_s3_poll_interval() -> u64 {
    300
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BundleUrlSourceConfig {
    #[serde(default = "default_bundle_download_timeout")]
    pub default_download_timeout_seconds: u64,
    #[serde(default = "default_verify_checksums")]
    pub verify_checksums: bool,
}

impl Default for BundleUrlSourceConfig {
    fn default() -> Self {
        Self {
            default_download_timeout_seconds: default_bundle_download_timeout(),
            verify_checksums: true,
        }
    }
}

fn default_bundle_download_timeout() -> u64 {
    60
}

fn default_verify_checksums() -> bool {
    true
}
