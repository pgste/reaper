//! S3 storage backend
//!
//! Stores bundles in Amazon S3 with metadata in object tags/headers.

use async_trait::async_trait;
use aws_sdk_s3::Client;
use chrono::Utc;
use tracing::{debug, info};

use super::traits::{BundleInfo, BundleMetadata, BundleStorage, StorageError, StoredBundle};

/// S3-based bundle storage
pub struct S3Storage {
    client: Client,
    bucket: String,
    prefix: Option<String>,
}

impl S3Storage {
    /// Create a new S3 storage
    pub async fn new(
        bucket: &str,
        region: &str,
        prefix: Option<&str>,
    ) -> Result<Self, StorageError> {
        let config = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .region(aws_sdk_s3::config::Region::new(region.to_string()))
            .load()
            .await;

        let client = Client::new(&config);

        info!(
            "Initialized S3 storage: bucket={}, region={}",
            bucket, region
        );

        Ok(Self {
            client,
            bucket: bucket.to_string(),
            prefix: prefix.map(|s| s.to_string()),
        })
    }

    /// Get the full S3 key for a bundle
    fn full_key(&self, key: &str) -> String {
        match &self.prefix {
            Some(prefix) => format!("{}/{}", prefix.trim_end_matches('/'), key),
            None => key.to_string(),
        }
    }

    /// Get the metadata key for a bundle
    fn metadata_key(&self, key: &str) -> String {
        format!("{}.meta.json", self.full_key(key))
    }
}

#[async_trait]
impl BundleStorage for S3Storage {
    async fn put(
        &self,
        key: &str,
        data: &[u8],
        metadata: BundleMetadata,
    ) -> Result<(), StorageError> {
        let full_key = self.full_key(key);
        let metadata_key = self.metadata_key(key);

        debug!("Storing bundle to S3: s3://{}/{}", self.bucket, full_key);

        // Upload bundle data
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(&full_key)
            .body(aws_sdk_s3::primitives::ByteStream::from(data.to_vec()))
            .content_type(&metadata.content_type)
            .send()
            .await
            .map_err(|e| StorageError::Operation(format!("S3 put failed: {}", e)))?;

        // Upload metadata as separate object
        let metadata_json = serde_json::to_string(&StoredMetadata {
            metadata,
            created_at: Utc::now(),
            size_bytes: data.len() as u64,
        })
        .map_err(|e| StorageError::Serialization(e.to_string()))?;

        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(&metadata_key)
            .body(aws_sdk_s3::primitives::ByteStream::from(
                metadata_json.into_bytes(),
            ))
            .content_type("application/json")
            .send()
            .await
            .map_err(|e| StorageError::Operation(format!("S3 metadata put failed: {}", e)))?;

        info!(
            "Stored bundle to S3: s3://{}/{} ({} bytes)",
            self.bucket,
            full_key,
            data.len()
        );

        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Option<StoredBundle>, StorageError> {
        let full_key = self.full_key(key);
        let metadata_key = self.metadata_key(key);

        debug!("Getting bundle from S3: s3://{}/{}", self.bucket, full_key);

        // Get bundle data
        let result = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(&full_key)
            .send()
            .await;

        let output = match result {
            Ok(output) => output,
            Err(e) => {
                // Check if it's a not found error
                if e.to_string().contains("NoSuchKey") || e.to_string().contains("not found") {
                    return Ok(None);
                }
                return Err(StorageError::Operation(format!("S3 get failed: {}", e)));
            }
        };

        let data = output
            .body
            .collect()
            .await
            .map_err(|e| StorageError::Operation(format!("S3 body read failed: {}", e)))?
            .to_vec();

        // Get metadata
        let metadata_output = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(&metadata_key)
            .send()
            .await
            .map_err(|e| StorageError::Operation(format!("S3 metadata get failed: {}", e)))?;

        let metadata_bytes = metadata_output
            .body
            .collect()
            .await
            .map_err(|e| StorageError::Operation(format!("S3 metadata body read failed: {}", e)))?
            .to_vec();

        let stored: StoredMetadata = serde_json::from_slice(&metadata_bytes)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;

        Ok(Some(StoredBundle {
            data,
            metadata: stored.metadata,
            created_at: stored.created_at,
            storage_key: key.to_string(),
        }))
    }

    async fn delete(&self, key: &str) -> Result<(), StorageError> {
        let full_key = self.full_key(key);
        let metadata_key = self.metadata_key(key);

        // Delete bundle
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(&full_key)
            .send()
            .await
            .map_err(|e| StorageError::Operation(format!("S3 delete failed: {}", e)))?;

        // Delete metadata
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(&metadata_key)
            .send()
            .await
            .map_err(|e| StorageError::Operation(format!("S3 metadata delete failed: {}", e)))?;

        info!("Deleted bundle from S3: s3://{}/{}", self.bucket, full_key);

        Ok(())
    }

    async fn list(&self, prefix: Option<&str>) -> Result<Vec<BundleInfo>, StorageError> {
        let search_prefix = match (&self.prefix, prefix) {
            (Some(base), Some(filter)) => format!("{}/{}", base.trim_end_matches('/'), filter),
            (Some(base), None) => base.clone(),
            (None, Some(filter)) => filter.to_string(),
            (None, None) => String::new(),
        };

        debug!(
            "Listing bundles from S3: s3://{}/{}",
            self.bucket, search_prefix
        );

        let mut bundles = Vec::new();
        let mut continuation_token: Option<String> = None;

        loop {
            let mut request = self
                .client
                .list_objects_v2()
                .bucket(&self.bucket)
                .prefix(&search_prefix);

            if let Some(token) = continuation_token {
                request = request.continuation_token(token);
            }

            let output = request
                .send()
                .await
                .map_err(|e| StorageError::Operation(format!("S3 list failed: {}", e)))?;

            for object in output.contents() {
                let key = object.key().unwrap_or_default();

                // Skip metadata files
                if key.ends_with(".meta.json") {
                    continue;
                }

                // Only include .rpp files
                if !key.ends_with(".rpp") {
                    continue;
                }

                // Try to get metadata
                let metadata_key = format!("{}.meta.json", key);
                if let Ok(metadata_output) = self
                    .client
                    .get_object()
                    .bucket(&self.bucket)
                    .key(&metadata_key)
                    .send()
                    .await
                {
                    if let Ok(metadata_bytes) = metadata_output.body.collect().await {
                        if let Ok(stored) =
                            serde_json::from_slice::<StoredMetadata>(&metadata_bytes.to_vec())
                        {
                            // Strip prefix from key
                            let relative_key = match &self.prefix {
                                Some(p) => key
                                    .strip_prefix(p.trim_end_matches('/'))
                                    .unwrap_or(key)
                                    .trim_start_matches('/')
                                    .to_string(),
                                None => key.to_string(),
                            };

                            bundles.push(BundleInfo {
                                key: relative_key,
                                size_bytes: stored.size_bytes,
                                created_at: stored.created_at,
                                metadata: stored.metadata,
                            });
                        }
                    }
                }
            }

            if output.is_truncated() == Some(true) {
                continuation_token = output.next_continuation_token().map(|s| s.to_string());
            } else {
                break;
            }
        }

        Ok(bundles)
    }

    async fn exists(&self, key: &str) -> Result<bool, StorageError> {
        let full_key = self.full_key(key);

        match self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(&full_key)
            .send()
            .await
        {
            Ok(_) => Ok(true),
            Err(e) => {
                if e.to_string().contains("NotFound") || e.to_string().contains("not found") {
                    Ok(false)
                } else {
                    Err(StorageError::Operation(format!("S3 head failed: {}", e)))
                }
            }
        }
    }

    fn backend_name(&self) -> &'static str {
        "s3"
    }
}

/// Internal structure for storing metadata
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct StoredMetadata {
    metadata: BundleMetadata,
    created_at: chrono::DateTime<chrono::Utc>,
    size_bytes: u64,
}
