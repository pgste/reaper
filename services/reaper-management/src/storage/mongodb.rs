//! MongoDB storage backend
//!
//! Stores bundles in MongoDB GridFS with metadata in document fields.

use async_trait::async_trait;
use chrono::Utc;
use mongodb::{
    bson::{doc, Bson},
    options::ClientOptions,
    Client,
};
use tracing::{debug, info};

use super::traits::{BundleInfo, BundleMetadata, BundleStorage, StorageError, StoredBundle};

/// MongoDB-based bundle storage
pub struct MongoDbStorage {
    client: Client,
    database: String,
    collection: String,
}

impl MongoDbStorage {
    /// Create a new MongoDB storage
    pub async fn new(uri: &str, database: &str, collection: &str) -> Result<Self, StorageError> {
        let client_options = ClientOptions::parse(uri)
            .await
            .map_err(|e| StorageError::Config(format!("MongoDB connection failed: {}", e)))?;

        let client = Client::with_options(client_options)
            .map_err(|e| StorageError::Config(format!("MongoDB client creation failed: {}", e)))?;

        info!(
            "Initialized MongoDB storage: database={}, collection={}",
            database, collection
        );

        Ok(Self {
            client,
            database: database.to_string(),
            collection: collection.to_string(),
        })
    }

    /// Get the collection
    fn get_collection(&self) -> mongodb::Collection<BundleDocument> {
        self.client
            .database(&self.database)
            .collection(&self.collection)
    }
}

#[async_trait]
impl BundleStorage for MongoDbStorage {
    async fn put(
        &self,
        key: &str,
        data: &[u8],
        metadata: BundleMetadata,
    ) -> Result<(), StorageError> {
        let collection = self.get_collection();

        debug!("Storing bundle to MongoDB: {}", key);

        let doc = BundleDocument {
            key: key.to_string(),
            data: mongodb::bson::Binary {
                subtype: mongodb::bson::spec::BinarySubtype::Generic,
                bytes: data.to_vec(),
            },
            metadata,
            created_at: Utc::now(),
            size_bytes: data.len() as u64,
        };

        // Upsert the document
        collection
            .replace_one(doc! { "key": key })
            .upsert(true)
            .send(doc)
            .await
            .map_err(|e| StorageError::Operation(format!("MongoDB put failed: {}", e)))?;

        info!("Stored bundle to MongoDB: {} ({} bytes)", key, data.len());

        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Option<StoredBundle>, StorageError> {
        let collection = self.get_collection();

        debug!("Getting bundle from MongoDB: {}", key);

        let result = collection
            .find_one(doc! { "key": key })
            .await
            .map_err(|e| StorageError::Operation(format!("MongoDB get failed: {}", e)))?;

        match result {
            Some(doc) => Ok(Some(StoredBundle {
                data: doc.data.bytes,
                metadata: doc.metadata,
                created_at: doc.created_at,
                storage_key: doc.key,
            })),
            None => Ok(None),
        }
    }

    async fn delete(&self, key: &str) -> Result<(), StorageError> {
        let collection = self.get_collection();

        collection
            .delete_one(doc! { "key": key })
            .await
            .map_err(|e| StorageError::Operation(format!("MongoDB delete failed: {}", e)))?;

        info!("Deleted bundle from MongoDB: {}", key);

        Ok(())
    }

    async fn list(&self, prefix: Option<&str>) -> Result<Vec<BundleInfo>, StorageError> {
        let collection = self.get_collection();

        let filter = match prefix {
            Some(p) => doc! { "key": { "$regex": format!("^{}", regex::escape(p)) } },
            None => doc! {},
        };

        let mut cursor = collection
            .find(filter)
            .await
            .map_err(|e| StorageError::Operation(format!("MongoDB list failed: {}", e)))?;

        let mut bundles = Vec::new();

        while cursor
            .advance()
            .await
            .map_err(|e| StorageError::Operation(format!("MongoDB cursor error: {}", e)))?
        {
            let doc = cursor.deserialize_current().map_err(|e| {
                StorageError::Operation(format!("MongoDB deserialize error: {}", e))
            })?;

            bundles.push(BundleInfo {
                key: doc.key,
                size_bytes: doc.size_bytes,
                created_at: doc.created_at,
                metadata: doc.metadata,
            });
        }

        Ok(bundles)
    }

    async fn exists(&self, key: &str) -> Result<bool, StorageError> {
        let collection = self.get_collection();

        let count = collection
            .count_documents(doc! { "key": key })
            .await
            .map_err(|e| StorageError::Operation(format!("MongoDB count failed: {}", e)))?;

        Ok(count > 0)
    }

    fn backend_name(&self) -> &'static str {
        "mongodb"
    }
}

/// Document structure for MongoDB storage
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct BundleDocument {
    key: String,
    data: mongodb::bson::Binary,
    metadata: BundleMetadata,
    created_at: chrono::DateTime<chrono::Utc>,
    size_bytes: u64,
}
