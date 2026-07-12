//! Database module for Reaper Management Server
//!
//! Provides database connection pooling and repository interfaces
//! supporting SQLite and PostgreSQL.

pub mod connection;
pub mod repositories;

pub use connection::{advisory_keys, AdvisoryLock, Database, DatabaseError};

use crate::config::DatabaseConfig;
use std::sync::Arc;

/// Initialize the database from configuration
pub async fn init_database(config: &DatabaseConfig) -> Result<Arc<Database>, DatabaseError> {
    let db = Database::new(config).await?;
    db.run_migrations().await?;
    Ok(Arc::new(db))
}

/// Build a `DatabaseConfig` for tests: SQLite in `dir` by default, or a
/// freshly-created PostgreSQL database when `REAPER_TEST_DATABASE_URL` is
/// set (pointing at an admin database, e.g. `postgres://…/postgres`).
/// Each call creates a uniquely-named database, so parallel tests stay
/// isolated exactly like per-tempdir SQLite files.
pub async fn ephemeral_test_config(dir: &std::path::Path) -> DatabaseConfig {
    if let Ok(base) = std::env::var("REAPER_TEST_DATABASE_URL") {
        connection::install_drivers();
        let name = format!("reaper_test_{}", uuid::Uuid::new_v4().simple());
        let admin = sqlx::AnyPool::connect(&base)
            .await
            .expect("connect to REAPER_TEST_DATABASE_URL admin database");
        sqlx::query(&format!("CREATE DATABASE {name}"))
            .execute(&admin)
            .await
            .expect("create ephemeral test database");
        admin.close().await;
        let (prefix, _) = base
            .rsplit_once('/')
            .expect("REAPER_TEST_DATABASE_URL must contain a database path");
        DatabaseConfig {
            db_type: "postgres".to_string(),
            url: format!("{prefix}/{name}"),
            replica_url: None,
            max_connections: 5,
        }
    } else {
        DatabaseConfig {
            db_type: "sqlite".to_string(),
            url: format!("sqlite:{}", dir.join("test.db").display()),
            replica_url: None,
            max_connections: 5,
        }
    }
}

/// Rewrite `?` placeholders as `$1..$n` so dynamically-assembled queries
/// stay portable across SQLite and PostgreSQL. Static queries are written
/// with `$n` directly; only builders that push fragments use this.
pub fn numbered_placeholders(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len() + 8);
    let mut n = 0u32;
    for ch in sql.chars() {
        if ch == '?' {
            n += 1;
            out.push('$');
            out.push_str(&n.to_string());
        } else {
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn numbered_placeholders_rewrites_in_order() {
        assert_eq!(
            super::numbered_placeholders("SELECT * FROM t WHERE a = ? AND b = ? LIMIT ?"),
            "SELECT * FROM t WHERE a = $1 AND b = $2 LIMIT $3"
        );
        assert_eq!(super::numbered_placeholders("no params"), "no params");
    }
}
