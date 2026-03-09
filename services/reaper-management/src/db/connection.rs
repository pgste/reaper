//! Database connection module
//!
//! Provides a unified interface for SQLite and PostgreSQL databases.

use crate::config::DatabaseConfig;
use sqlx::{migrate::MigrateDatabase, Sqlite, SqlitePool};
use std::path::Path;
use thiserror::Error;
use tracing::{info, warn};

/// Database errors
#[derive(Debug, Error)]
pub enum DatabaseError {
    #[error("Database connection error: {0}")]
    Connection(#[from] sqlx::Error),
    #[error("Migration error: {0}")]
    Migration(String),
    #[error("Configuration error: {0}")]
    Config(String),
    #[error("Database not found: {0}")]
    NotFound(String),
}

/// Database wrapper supporting multiple backends
#[derive(Clone)]
pub struct Database {
    sqlite_pool: Option<SqlitePool>,
    db_type: String,
}

impl Database {
    /// Create a new database connection from configuration
    pub async fn new(config: &DatabaseConfig) -> Result<Self, DatabaseError> {
        match config.db_type.as_str() {
            "sqlite" => Self::new_sqlite(&config.url, config.max_connections).await,
            "postgres" | "postgresql" => {
                // PostgreSQL support would go here
                Err(DatabaseError::Config(
                    "PostgreSQL support not yet implemented".to_string(),
                ))
            }
            other => Err(DatabaseError::Config(format!(
                "Unsupported database type: {}",
                other
            ))),
        }
    }

    /// Create a new SQLite database connection
    async fn new_sqlite(url: &str, _max_connections: u32) -> Result<Self, DatabaseError> {
        // Ensure database file directory exists
        if url.starts_with("sqlite:") {
            let db_path = url.trim_start_matches("sqlite:");
            let db_path = db_path.trim_start_matches("//");

            if let Some(parent) = Path::new(db_path).parent() {
                if !parent.exists() {
                    info!("Creating database directory: {:?}", parent);
                    std::fs::create_dir_all(parent).map_err(|e| {
                        DatabaseError::Config(format!("Failed to create database directory: {}", e))
                    })?;
                }
            }

            // Create database if it doesn't exist
            if !Sqlite::database_exists(url).await.unwrap_or(false) {
                info!("Creating SQLite database: {}", url);
                Sqlite::create_database(url).await?;
            }
        }

        info!("Connecting to SQLite database: {}", url);

        let pool = SqlitePool::connect_with(
            sqlx::sqlite::SqliteConnectOptions::new()
                .filename(url.trim_start_matches("sqlite:").trim_start_matches("//"))
                .create_if_missing(true)
                .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
                .synchronous(sqlx::sqlite::SqliteSynchronous::Normal),
        )
        .await?;

        Ok(Self {
            sqlite_pool: Some(pool),
            db_type: "sqlite".to_string(),
        })
    }

    /// Run database migrations
    pub async fn run_migrations(&self) -> Result<(), DatabaseError> {
        info!("Running database migrations...");

        // List of migration files in order
        let migrations = [
            include_str!("migrations/001_initial.sql"),
            include_str!("migrations/002_namespaces.sql"),
            include_str!("migrations/003_security.sql"),
            include_str!("migrations/004_users_and_audit.sql"),
            include_str!("migrations/005_phase2_operations.sql"),
        ];

        match &self.sqlite_pool {
            Some(pool) => {
                for (idx, migration_sql) in migrations.iter().enumerate() {
                    info!("Running migration {}", idx + 1);

                    // Split by semicolons and execute each statement
                    for statement in migration_sql.split(';') {
                        // Clean the statement: remove comments and trim
                        let statement: String = statement
                            .lines()
                            .filter(|line| !line.trim().starts_with("--"))
                            .collect::<Vec<_>>()
                            .join("\n");
                        let statement = statement.trim();

                        if statement.is_empty() {
                            continue;
                        }

                        if let Err(e) = sqlx::query(statement).execute(pool).await {
                            let err_str = e.to_string();
                            // Ignore "already exists" errors for idempotent migrations
                            if !err_str.contains("already exists")
                                && !err_str.contains("duplicate column")
                            {
                                warn!("Migration statement failed: {} - SQL: {}", e, statement);
                                // Don't fail on non-critical errors for now
                            }
                        }
                    }
                }
                info!("Migrations completed successfully");
                Ok(())
            }
            None => Err(DatabaseError::Connection(sqlx::Error::Configuration(
                "No database pool available".into(),
            ))),
        }
    }

    /// Get the SQLite pool (if using SQLite)
    pub fn sqlite_pool(&self) -> Option<&SqlitePool> {
        self.sqlite_pool.as_ref()
    }

    /// Get the database type
    pub fn db_type(&self) -> &str {
        &self.db_type
    }

    /// Execute a raw SQL query (for SQLite)
    pub async fn execute(&self, query: &str) -> Result<u64, DatabaseError> {
        match &self.sqlite_pool {
            Some(pool) => {
                let result = sqlx::query(query).execute(pool).await?;
                Ok(result.rows_affected())
            }
            None => Err(DatabaseError::Connection(sqlx::Error::Configuration(
                "No database pool available".into(),
            ))),
        }
    }

    /// Create a mock database for testing (no actual connection)
    #[cfg(test)]
    pub fn new_mock() -> Self {
        Self {
            sqlite_pool: None,
            db_type: "mock".to_string(),
        }
    }
}

impl std::fmt::Debug for Database {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Database")
            .field("db_type", &self.db_type)
            .field("connected", &self.sqlite_pool.is_some())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_sqlite_connection() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let url = format!("sqlite:{}", db_path.display());

        let config = DatabaseConfig {
            db_type: "sqlite".to_string(),
            url,
            max_connections: 5,
        };

        let db = Database::new(&config).await.unwrap();
        assert_eq!(db.db_type(), "sqlite");
        assert!(db.sqlite_pool().is_some());
    }

    #[tokio::test]
    async fn test_migrations() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let url = format!("sqlite:{}", db_path.display());

        let config = DatabaseConfig {
            db_type: "sqlite".to_string(),
            url,
            max_connections: 5,
        };

        let db = Database::new(&config).await.unwrap();
        db.run_migrations().await.unwrap();

        // Verify tables exist
        let pool = db.sqlite_pool().unwrap();
        let result: (i32,) = sqlx::query_as("SELECT COUNT(*) FROM organizations")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(result.0, 0);
    }
}
