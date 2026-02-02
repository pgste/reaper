//! Storage configuration for compiled bundles

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use super::error::ConfigError;

/// Storage configuration for compiled bundles
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StorageConfig {
    #[serde(default = "default_storage_type", rename = "type")]
    pub storage_type: String,
    #[serde(default)]
    pub filesystem: FilesystemStorageConfig,
    #[serde(default)]
    pub s3: S3StorageConfig,
    #[serde(default)]
    pub mongodb: MongoDbStorageConfig,
    #[serde(default)]
    pub dynamodb: DynamoDbStorageConfig,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            storage_type: default_storage_type(),
            filesystem: FilesystemStorageConfig::default(),
            s3: S3StorageConfig::default(),
            mongodb: MongoDbStorageConfig::default(),
            dynamodb: DynamoDbStorageConfig::default(),
        }
    }
}

impl StorageConfig {
    /// Validate storage configuration
    pub fn validate(&self) -> Result<(), ConfigError> {
        match self.storage_type.as_str() {
            "filesystem" => {
                // Filesystem storage validated later in prepare_directories
                Ok(())
            }
            "s3" => {
                if self.s3.bucket.is_none() {
                    return Err(ConfigError::S3MissingBucket);
                }
                Ok(())
            }
            "mongodb" => {
                if self.mongodb.uri.is_none() {
                    return Err(ConfigError::MongoDbMissingUri);
                }
                Ok(())
            }
            "dynamodb" => {
                if self.dynamodb.table.is_none() {
                    return Err(ConfigError::DynamoDbMissingTable);
                }
                Ok(())
            }
            other => Err(ConfigError::UnsupportedStorageType(other.to_string())),
        }
    }
}

fn default_storage_type() -> String {
    "filesystem".to_string()
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FilesystemStorageConfig {
    #[serde(default = "default_storage_path")]
    pub path: PathBuf,
}

impl Default for FilesystemStorageConfig {
    fn default() -> Self {
        Self {
            path: default_storage_path(),
        }
    }
}

pub(super) fn default_storage_path() -> PathBuf {
    PathBuf::from("/var/lib/reaper/bundles")
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct S3StorageConfig {
    pub bucket: Option<String>,
    pub region: Option<String>,
    pub prefix: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct MongoDbStorageConfig {
    pub uri: Option<String>,
    pub database: Option<String>,
    pub collection: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct DynamoDbStorageConfig {
    pub table: Option<String>,
    pub region: Option<String>,
}
