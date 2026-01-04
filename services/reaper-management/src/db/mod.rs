//! Database module for Reaper Management Server
//!
//! Provides database connection pooling and repository interfaces
//! supporting SQLite and PostgreSQL.

pub mod connection;
pub mod repositories;

pub use connection::{Database, DatabaseError};

use crate::config::DatabaseConfig;
use std::sync::Arc;

/// Initialize the database from configuration
pub async fn init_database(config: &DatabaseConfig) -> Result<Arc<Database>, DatabaseError> {
    let db = Database::new(config).await?;
    db.run_migrations().await?;
    Ok(Arc::new(db))
}
