//! DynamoDB storage backend
//!
//! Stores bundles in AWS DynamoDB with binary data in item attributes.
//! Note: DynamoDB has a 400KB item size limit, so large bundles may need
//! to be split or stored in S3 with only metadata in DynamoDB.

use async_trait::async_trait;
use aws_sdk_dynamodb::{types::AttributeValue, Client};
use chrono::Utc;
use tracing::{debug, info};

use super::traits::{BundleInfo, BundleMetadata, BundleStorage, StorageError, StoredBundle};

/// DynamoDB-based bundle storage
pub struct DynamoDbStorage {
    client: Client,
    table_name: String,
}

impl DynamoDbStorage {
    /// Create a new DynamoDB storage
    pub async fn new(table_name: &str, region: &str) -> Result<Self, StorageError> {
        let config = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .region(aws_sdk_dynamodb::config::Region::new(region.to_string()))
            .load()
            .await;

        let client = Client::new(&config);

        info!(
            "Initialized DynamoDB storage: table={}, region={}",
            table_name, region
        );

        Ok(Self {
            client,
            table_name: table_name.to_string(),
        })
    }
}

#[async_trait]
impl BundleStorage for DynamoDbStorage {
    async fn put(
        &self,
        key: &str,
        data: &[u8],
        metadata: BundleMetadata,
    ) -> Result<(), StorageError> {
        debug!("Storing bundle to DynamoDB: {}", key);

        // Check size limit (400KB for DynamoDB items)
        if data.len() > 350_000 {
            return Err(StorageError::Operation(
                "Bundle too large for DynamoDB (max ~350KB). Consider using S3 storage.".to_string(),
            ));
        }

        let metadata_json = serde_json::to_string(&metadata)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;

        let now = Utc::now().to_rfc3339();

        self.client
            .put_item()
            .table_name(&self.table_name)
            .item("pk", AttributeValue::S(key.to_string()))
            .item("data", AttributeValue::B(aws_sdk_dynamodb::primitives::Blob::new(data.to_vec())))
            .item("metadata", AttributeValue::S(metadata_json))
            .item("created_at", AttributeValue::S(now))
            .item("size_bytes", AttributeValue::N(data.len().to_string()))
            .send()
            .await
            .map_err(|e| StorageError::Operation(format!("DynamoDB put failed: {}", e)))?;

        info!(
            "Stored bundle to DynamoDB: {} ({} bytes)",
            key,
            data.len()
        );

        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Option<StoredBundle>, StorageError> {
        debug!("Getting bundle from DynamoDB: {}", key);

        let result = self
            .client
            .get_item()
            .table_name(&self.table_name)
            .key("pk", AttributeValue::S(key.to_string()))
            .send()
            .await
            .map_err(|e| StorageError::Operation(format!("DynamoDB get failed: {}", e)))?;

        match result.item {
            Some(item) => {
                let data = item
                    .get("data")
                    .and_then(|v| v.as_b().ok())
                    .map(|b| b.as_ref().to_vec())
                    .ok_or_else(|| StorageError::Operation("Missing data field".to_string()))?;

                let metadata_json = item
                    .get("metadata")
                    .and_then(|v| v.as_s().ok())
                    .ok_or_else(|| StorageError::Operation("Missing metadata field".to_string()))?;

                let metadata: BundleMetadata = serde_json::from_str(metadata_json)
                    .map_err(|e| StorageError::Serialization(e.to_string()))?;

                let created_at = item
                    .get("created_at")
                    .and_then(|v| v.as_s().ok())
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(Utc::now);

                Ok(Some(StoredBundle {
                    data,
                    metadata,
                    created_at,
                    storage_key: key.to_string(),
                }))
            }
            None => Ok(None),
        }
    }

    async fn delete(&self, key: &str) -> Result<(), StorageError> {
        self.client
            .delete_item()
            .table_name(&self.table_name)
            .key("pk", AttributeValue::S(key.to_string()))
            .send()
            .await
            .map_err(|e| StorageError::Operation(format!("DynamoDB delete failed: {}", e)))?;

        info!("Deleted bundle from DynamoDB: {}", key);

        Ok(())
    }

    async fn list(&self, prefix: Option<&str>) -> Result<Vec<BundleInfo>, StorageError> {
        debug!("Listing bundles from DynamoDB with prefix: {:?}", prefix);

        let mut bundles = Vec::new();
        let mut last_evaluated_key = None;

        loop {
            let mut request = self.client.scan().table_name(&self.table_name);

            if let Some(prefix) = prefix {
                request = request
                    .filter_expression("begins_with(pk, :prefix)")
                    .expression_attribute_values(":prefix", AttributeValue::S(prefix.to_string()));
            }

            if let Some(key) = last_evaluated_key {
                request = request.set_exclusive_start_key(Some(key));
            }

            let result = request
                .send()
                .await
                .map_err(|e| StorageError::Operation(format!("DynamoDB scan failed: {}", e)))?;

            if let Some(items) = result.items {
                for item in items {
                    let key = item
                        .get("pk")
                        .and_then(|v| v.as_s().ok())
                        .cloned()
                        .unwrap_or_default();

                    let size_bytes = item
                        .get("size_bytes")
                        .and_then(|v| v.as_n().ok())
                        .and_then(|n| n.parse().ok())
                        .unwrap_or(0);

                    let created_at = item
                        .get("created_at")
                        .and_then(|v| v.as_s().ok())
                        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(Utc::now);

                    let metadata = item
                        .get("metadata")
                        .and_then(|v| v.as_s().ok())
                        .and_then(|s| serde_json::from_str(s).ok())
                        .unwrap_or_else(|| BundleMetadata::new(
                            uuid::Uuid::nil(),
                            uuid::Uuid::nil(),
                            "unknown".to_string(),
                            0,
                            "".to_string(),
                        ));

                    bundles.push(BundleInfo {
                        key,
                        size_bytes,
                        created_at,
                        metadata,
                    });
                }
            }

            last_evaluated_key = result.last_evaluated_key;
            if last_evaluated_key.is_none() {
                break;
            }
        }

        Ok(bundles)
    }

    async fn exists(&self, key: &str) -> Result<bool, StorageError> {
        let result = self
            .client
            .get_item()
            .table_name(&self.table_name)
            .key("pk", AttributeValue::S(key.to_string()))
            .projection_expression("pk")
            .send()
            .await
            .map_err(|e| StorageError::Operation(format!("DynamoDB get failed: {}", e)))?;

        Ok(result.item.is_some())
    }

    fn backend_name(&self) -> &'static str {
        "dynamodb"
    }
}
