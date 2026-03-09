//! Storage backends for compiled policy bundles
//!
//! Provides pluggable storage with multiple backend implementations:
//! - Filesystem (default)
//! - S3 (with `storage-s3` feature)
//! - MongoDB (with `storage-mongodb` feature)
//! - DynamoDB (with `storage-dynamodb` feature)

pub mod filesystem;
pub mod traits;

#[cfg(feature = "storage-s3")]
pub mod s3;

#[cfg(feature = "storage-mongodb")]
pub mod mongodb;

#[cfg(feature = "storage-dynamodb")]
pub mod dynamodb;

pub use filesystem::FilesystemStorage;
pub use traits::{BundleMetadata, BundleStorage, StorageError, StoredBundle};

#[cfg(feature = "storage-s3")]
pub use s3::S3Storage;

#[cfg(feature = "storage-mongodb")]
pub use mongodb::MongoDbStorage;

#[cfg(feature = "storage-dynamodb")]
pub use dynamodb::DynamoDbStorage;

use crate::config::StorageConfig;
use std::sync::Arc;

/// Create a storage backend from configuration
pub async fn create_storage(
    config: &StorageConfig,
) -> Result<Arc<dyn BundleStorage>, StorageError> {
    match config.storage_type.as_str() {
        "filesystem" => {
            let storage = FilesystemStorage::new(&config.filesystem.path)?;
            Ok(Arc::new(storage))
        }
        #[cfg(feature = "storage-s3")]
        "s3" => {
            let bucket = config
                .s3
                .bucket
                .as_ref()
                .ok_or_else(|| StorageError::Config("S3 bucket not configured".to_string()))?;
            let region = config
                .s3
                .region
                .as_ref()
                .ok_or_else(|| StorageError::Config("S3 region not configured".to_string()))?;
            let storage = S3Storage::new(bucket, region, config.s3.prefix.as_deref()).await?;
            Ok(Arc::new(storage))
        }
        #[cfg(feature = "storage-mongodb")]
        "mongodb" => {
            let uri =
                config.mongodb.uri.as_ref().ok_or_else(|| {
                    StorageError::Config("MongoDB URI not configured".to_string())
                })?;
            let database = config.mongodb.database.as_ref().ok_or_else(|| {
                StorageError::Config("MongoDB database not configured".to_string())
            })?;
            let collection = config.mongodb.collection.as_deref().unwrap_or("bundles");
            let storage = MongoDbStorage::new(uri, database, collection).await?;
            Ok(Arc::new(storage))
        }
        #[cfg(feature = "storage-dynamodb")]
        "dynamodb" => {
            let table =
                config.dynamodb.table.as_ref().ok_or_else(|| {
                    StorageError::Config("DynamoDB table not configured".to_string())
                })?;
            let region = config.dynamodb.region.as_ref().ok_or_else(|| {
                StorageError::Config("DynamoDB region not configured".to_string())
            })?;
            let storage = DynamoDbStorage::new(table, region).await?;
            Ok(Arc::new(storage))
        }
        other => Err(StorageError::Config(format!(
            "Unsupported storage type: {}. Available: filesystem{}{}{}",
            other,
            if cfg!(feature = "storage-s3") {
                ", s3"
            } else {
                ""
            },
            if cfg!(feature = "storage-mongodb") {
                ", mongodb"
            } else {
                ""
            },
            if cfg!(feature = "storage-dynamodb") {
                ", dynamodb"
            } else {
                ""
            },
        ))),
    }
}
