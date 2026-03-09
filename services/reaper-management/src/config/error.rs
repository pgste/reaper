//! Configuration error types

use thiserror::Error;

/// Configuration validation errors
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("Invalid port: {0}. Must be between 1 and 65535")]
    InvalidPort(u16),
    #[error("Invalid bind address: {0}")]
    InvalidBindAddress(String),
    #[error("Invalid database URL: {0}")]
    InvalidDatabaseUrl(String),
    #[error("Unsupported database type: {0}. Supported: sqlite, postgres")]
    UnsupportedDatabaseType(String),
    #[error("Unsupported storage type: {0}. Supported: filesystem, s3, mongodb, dynamodb")]
    UnsupportedStorageType(String),
    #[error("Missing required configuration: {0}")]
    MissingRequired(String),
    #[error("Invalid timeout value: {0}. Must be positive")]
    InvalidTimeout(String),
    #[error("Invalid rate limit: {0}. Must be positive")]
    InvalidRateLimit(String),
    #[error("Path does not exist: {0}")]
    PathNotFound(String),
    #[error("Path is not writable: {0}")]
    PathNotWritable(String),
    #[error("JWT secret too short: minimum 32 characters required")]
    JwtSecretTooShort,
    #[error("S3 storage requires bucket name")]
    S3MissingBucket,
    #[error("MongoDB storage requires URI")]
    MongoDbMissingUri,
    #[error("DynamoDB storage requires table name")]
    DynamoDbMissingTable,
}
