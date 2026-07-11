//! Database connection module
//!
//! One `Database` type over sqlx's `Any` driver, so every repository runs
//! the SAME query codebase (all `$n` placeholders) against SQLite (default,
//! zero-config) or PostgreSQL (production / self-hosted mode, selected via
//! `REAPER_DATABASE_TYPE=postgres`).
//!
//! Schema management:
//! - SQLite: legacy idempotent runner over the per-feature migration files
//!   (`migrations/00X_*.sql`), preserved for painless local upgrades.
//! - PostgreSQL: embedded, versioned, checksummed migrator. Each migration
//!   runs in its own transaction (PostgreSQL DDL is transactional) and is
//!   recorded in `_reaper_migrations`; a checksum mismatch on an
//!   already-applied version is a HARD error (schema drift refusal).

use crate::config::DatabaseConfig;
use sha2::{Digest, Sha256};
use sqlx::any::{AnyPoolOptions, AnyRow};
use sqlx::{migrate::MigrateDatabase, AnyPool, Row, Sqlite};
use std::path::Path;
use std::sync::Once;
use thiserror::Error;
use tracing::{info, warn};

/// Embedded PostgreSQL migrations: (version, description, sql).
/// APPEND-ONLY — never edit a shipped migration (the checksum guard will
/// refuse to start against a database that applied the old text).
const PG_MIGRATIONS: &[(i64, &str, &str)] = &[
    (
        1,
        "initial_schema",
        include_str!("migrations_pg/0001_initial_schema.sql"),
    ),
    (
        2,
        "change_log_retention",
        include_str!("migrations_pg/0002_change_log_retention.sql"),
    ),
    (
        3,
        "revocations",
        include_str!("migrations_pg/0003_revocations.sql"),
    ),
    (
        4,
        "promotion_change_requests",
        include_str!("migrations_pg/0004_promotion_change_requests.sql"),
    ),
    (5, "sso", include_str!("migrations_pg/0005_sso.sql")),
    (6, "scim", include_str!("migrations_pg/0006_scim.sql")),
    (
        7,
        "audit_governance",
        include_str!("migrations_pg/0007_audit_governance.sql"),
    ),
    (
        8,
        "idempotency_keys",
        include_str!("migrations_pg/0008_idempotency_keys.sql"),
    ),
];

static INSTALL_DRIVERS: Once = Once::new();

pub(crate) fn install_drivers() {
    INSTALL_DRIVERS.call_once(sqlx::any::install_default_drivers);
}

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
    /// Optimistic-concurrency guard failed: the row's version/timestamp no
    /// longer matches what the caller read (a concurrent writer won). Maps to
    /// HTTP 412 Precondition Failed at the API layer (Plan 07 Phase C).
    #[error("Version conflict: {0}")]
    VersionConflict(String),
}

/// Database wrapper supporting multiple backends
#[derive(Clone)]
pub struct Database {
    pool: Option<AnyPool>,
    db_type: String,
}

impl Database {
    /// Create a new database connection from configuration
    pub async fn new(config: &DatabaseConfig) -> Result<Self, DatabaseError> {
        install_drivers();
        match config.db_type.as_str() {
            "sqlite" => Self::new_sqlite(&config.url, config.max_connections).await,
            "postgres" | "postgresql" => {
                Self::new_postgres(&config.url, config.max_connections).await
            }
            other => Err(DatabaseError::Config(format!(
                "Unsupported database type: {}",
                other
            ))),
        }
    }

    /// Create a new SQLite database connection
    async fn new_sqlite(url: &str, max_connections: u32) -> Result<Self, DatabaseError> {
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

        let pool = AnyPoolOptions::new()
            .max_connections(max_connections)
            .after_connect(|conn, _meta| {
                Box::pin(async move {
                    // WAL: readers never block the writer (and vice versa).
                    // synchronous=NORMAL: fsync at checkpoint, not per-commit
                    // — the right durability/latency point for a WAL DB.
                    // busy_timeout: writers WAIT briefly under contention
                    // instead of failing with SQLITE_BUSY — control-plane
                    // saves must be reliable, and at UI/API rates a rare
                    // few-ms wait is invisible.
                    for pragma in [
                        "PRAGMA journal_mode = WAL",
                        "PRAGMA synchronous = NORMAL",
                        "PRAGMA busy_timeout = 5000",
                        "PRAGMA foreign_keys = ON",
                    ] {
                        sqlx::query(pragma).execute(&mut *conn).await?;
                    }
                    Ok(())
                })
            })
            .connect(url)
            .await?;

        Ok(Self {
            pool: Some(pool),
            db_type: "sqlite".to_string(),
        })
    }

    /// Create a new PostgreSQL database connection
    async fn new_postgres(url: &str, max_connections: u32) -> Result<Self, DatabaseError> {
        info!("Connecting to PostgreSQL database");

        let pool = AnyPoolOptions::new()
            .max_connections(max_connections)
            .acquire_timeout(std::time::Duration::from_secs(10))
            .connect(url)
            .await?;

        Ok(Self {
            pool: Some(pool),
            db_type: "postgres".to_string(),
        })
    }

    /// Run database migrations
    pub async fn run_migrations(&self) -> Result<(), DatabaseError> {
        let pool = self.pool.as_ref().ok_or_else(|| {
            DatabaseError::Connection(sqlx::Error::Configuration(
                "No database pool available".into(),
            ))
        })?;
        match self.db_type.as_str() {
            "postgres" => self.run_pg_migrations(pool).await,
            _ => self.run_sqlite_migrations(pool).await,
        }
    }

    /// Legacy idempotent runner for SQLite (re-executes every file; errors
    /// like "already exists" are expected and skipped).
    async fn run_sqlite_migrations(&self, pool: &AnyPool) -> Result<(), DatabaseError> {
        info!("Running database migrations (sqlite)...");

        // List of migration files in order
        let migrations = [
            include_str!("migrations/001_initial.sql"),
            include_str!("migrations/002_namespaces.sql"),
            include_str!("migrations/003_security.sql"),
            include_str!("migrations/004_users_and_audit.sql"),
            include_str!("migrations/005_phase2_operations.sql"),
            include_str!("migrations/006_data_plane.sql"),
            include_str!("migrations/007_change_log.sql"),
            include_str!("migrations/008_agent_data_sync.sql"),
            include_str!("migrations/009_change_log_retention.sql"),
            include_str!("migrations/010_revocations.sql"),
            include_str!("migrations/011_promotion_change_requests.sql"),
            include_str!("migrations/012_sso.sql"),
            include_str!("migrations/013_scim.sql"),
            include_str!("migrations/014_audit_governance.sql"),
            include_str!("migrations/015_idempotency_keys.sql"),
        ];

        for (idx, migration_sql) in migrations.iter().enumerate() {
            info!("Running migration {}", idx + 1);

            for statement in split_sql_statements(migration_sql) {
                if let Err(e) = sqlx::query(&statement).execute(pool).await {
                    let err_str = e.to_string();
                    // Ignore "already exists" errors for idempotent migrations
                    if !err_str.contains("already exists") && !err_str.contains("duplicate column")
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

    /// Versioned, checksummed migrator for PostgreSQL.
    async fn run_pg_migrations(&self, pool: &AnyPool) -> Result<(), DatabaseError> {
        info!("Running database migrations (postgres)...");

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS _reaper_migrations (
                version BIGINT PRIMARY KEY,
                description TEXT NOT NULL,
                checksum TEXT NOT NULL,
                applied_at TIMESTAMPTZ NOT NULL DEFAULT now()
            )",
        )
        .execute(pool)
        .await
        .map_err(|e| DatabaseError::Migration(format!("create _reaper_migrations: {e}")))?;

        for (version, description, sql) in PG_MIGRATIONS {
            let checksum = hex::encode(Sha256::digest(sql.as_bytes()));

            let applied: Option<AnyRow> =
                sqlx::query("SELECT checksum FROM _reaper_migrations WHERE version = $1")
                    .bind(version)
                    .fetch_optional(pool)
                    .await
                    .map_err(|e| DatabaseError::Migration(format!("read migration state: {e}")))?;

            if let Some(row) = applied {
                let existing: String = row.get("checksum");
                if existing != checksum {
                    // Schema drift: the shipped migration text no longer
                    // matches what this database applied. Refusing to start
                    // beats silently diverging schemas.
                    return Err(DatabaseError::Migration(format!(
                        "migration {version} ({description}) checksum mismatch: \
                         applied={existing} current={checksum} — migrations are \
                         append-only; write a new version instead of editing"
                    )));
                }
                continue;
            }

            info!("Applying PG migration {} ({})", version, description);
            let mut tx = pool
                .begin()
                .await
                .map_err(|e| DatabaseError::Migration(format!("begin migration tx: {e}")))?;
            for statement in split_sql_statements(sql) {
                sqlx::query(&statement)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| {
                        DatabaseError::Migration(format!(
                            "migration {version} failed: {e} — SQL: {statement}"
                        ))
                    })?;
            }
            sqlx::query(
                "INSERT INTO _reaper_migrations (version, description, checksum) \
                 VALUES ($1, $2, $3)",
            )
            .bind(version)
            .bind(*description)
            .bind(&checksum)
            .execute(&mut *tx)
            .await
            .map_err(|e| DatabaseError::Migration(format!("record migration {version}: {e}")))?;
            tx.commit().await.map_err(|e| {
                DatabaseError::Migration(format!("commit migration {version}: {e}"))
            })?;
        }

        info!("Migrations completed successfully");
        Ok(())
    }

    /// Get the connection pool (SQLite or PostgreSQL via sqlx::Any)
    pub fn any_pool(&self) -> Option<&AnyPool> {
        self.pool.as_ref()
    }

    /// Get the database type
    pub fn db_type(&self) -> &str {
        &self.db_type
    }

    /// Execute a raw SQL query
    pub async fn execute(&self, query: &str) -> Result<u64, DatabaseError> {
        match &self.pool {
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
            pool: None,
            db_type: "mock".to_string(),
        }
    }
}

/// Split a migration file into individual statements: `;` terminates a
/// statement unless it appears inside a single-quoted SQL string. Line
/// comments (`-- ...`) are stripped so a `;` in a comment can't split a
/// statement in half.
fn split_sql_statements(sql: &str) -> Vec<String> {
    // Strip line comments first (outside of quotes).
    let mut cleaned = String::with_capacity(sql.len());
    for line in sql.lines() {
        let mut in_quote = false;
        let mut cut = line.len();
        let bytes = line.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'\'' => in_quote = !in_quote,
                b'-' if !in_quote && i + 1 < bytes.len() && bytes[i + 1] == b'-' => {
                    cut = i;
                    break;
                }
                _ => {}
            }
            i += 1;
        }
        cleaned.push_str(&line[..cut]);
        cleaned.push('\n');
    }

    let mut statements = Vec::new();
    let mut current = String::new();
    let mut in_quote = false;
    for ch in cleaned.chars() {
        match ch {
            '\'' => {
                in_quote = !in_quote;
                current.push(ch);
            }
            ';' if !in_quote => {
                let stmt = current.trim().to_string();
                if !stmt.is_empty() {
                    statements.push(stmt);
                }
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    let tail = current.trim().to_string();
    if !tail.is_empty() {
        statements.push(tail);
    }
    statements
}

impl std::fmt::Debug for Database {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Database")
            .field("db_type", &self.db_type)
            .field("connected", &self.pool.is_some())
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
        assert!(db.any_pool().is_some());
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
        let pool = db.any_pool().unwrap();
        let result: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM organizations")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(result.0, 0);
    }

    #[test]
    fn test_split_sql_statements() {
        let sql = "-- comment with ; semicolon\nCREATE TABLE t (id TEXT); \
                   INSERT INTO t VALUES ('a;b'); -- trailing ; comment\n";
        let stmts = split_sql_statements(sql);
        assert_eq!(stmts.len(), 2);
        assert_eq!(stmts[0], "CREATE TABLE t (id TEXT)");
        assert_eq!(stmts[1], "INSERT INTO t VALUES ('a;b')");
    }

    #[test]
    fn test_pg_migrations_are_append_only_and_ordered() {
        let mut prev = 0;
        for (version, _, sql) in PG_MIGRATIONS {
            assert!(*version > prev, "versions must be strictly increasing");
            prev = *version;
            assert!(!sql.trim().is_empty(), "migration {} is empty", version);
        }
    }
}
