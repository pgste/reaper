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
