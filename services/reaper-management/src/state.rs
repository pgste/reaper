//! Application state for Reaper Management Server
//!
//! Holds shared state accessible to all request handlers.

use crate::bundle::BundleService;
use crate::config::Config;
use crate::db::Database;
use crate::storage::BundleStorage;
use std::sync::Arc;
use tokio::sync::broadcast;

/// Event types for SSE broadcasting
#[derive(Debug, Clone)]
pub enum ServerEvent {
    /// Policy was updated
    PolicyUpdated {
        policy_id: uuid::Uuid,
        org_id: uuid::Uuid,
        version: i32,
    },
    /// Policy was deleted
    PolicyDeleted {
        policy_id: uuid::Uuid,
        org_id: uuid::Uuid,
    },
    /// Bundle was promoted
    BundlePromoted {
        bundle_id: uuid::Uuid,
        org_id: uuid::Uuid,
        version: String,
        download_url: String,
    },
    /// Bundle was staged
    BundleStaged {
        bundle_id: uuid::Uuid,
        org_id: uuid::Uuid,
    },
    /// Data source refresh notification
    DataRefresh {
        source_id: uuid::Uuid,
        org_id: uuid::Uuid,
        source_type: String,
    },
    /// Keep-alive ping
    Ping {
        timestamp: chrono::DateTime<chrono::Utc>,
    },
}

/// Shared application state
#[derive(Clone)]
pub struct AppState {
    /// Database connection
    pub db: Arc<Database>,
    /// Configuration
    pub config: Arc<Config>,
    /// Bundle service for compilation and promotion
    pub bundle_service: Arc<BundleService>,
    /// Event broadcaster for SSE
    pub event_tx: broadcast::Sender<ServerEvent>,
    /// Server start time
    pub started_at: chrono::DateTime<chrono::Utc>,
}

impl AppState {
    /// Create new application state
    pub fn new(db: Arc<Database>, config: Config, storage: Arc<dyn BundleStorage>) -> Self {
        let (event_tx, _) = broadcast::channel(1024);
        let bundle_service = Arc::new(BundleService::new(db.clone(), storage));

        Self {
            db,
            config: Arc::new(config),
            bundle_service,
            event_tx,
            started_at: chrono::Utc::now(),
        }
    }

    /// Get a new event receiver for SSE connections
    pub fn subscribe_events(&self) -> broadcast::Receiver<ServerEvent> {
        self.event_tx.subscribe()
    }

    /// Broadcast an event to all connected clients
    pub fn broadcast_event(&self, event: ServerEvent) {
        // Ignore send errors (no subscribers)
        let _ = self.event_tx.send(event);
    }

    /// Get server uptime in seconds
    pub fn uptime_seconds(&self) -> i64 {
        chrono::Utc::now()
            .signed_duration_since(self.started_at)
            .num_seconds()
    }
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("db", &self.db)
            .field("started_at", &self.started_at)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DatabaseConfig;
    use crate::storage::FilesystemStorage;

    #[tokio::test]
    async fn test_event_broadcast() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let storage_path = temp_dir.path().join("storage");
        std::fs::create_dir_all(&storage_path).unwrap();
        let url = format!("sqlite:{}", db_path.display());

        let db_config = DatabaseConfig {
            db_type: "sqlite".to_string(),
            url,
            max_connections: 5,
        };

        let db = Database::new(&db_config).await.unwrap();
        db.run_migrations().await.unwrap();
        let storage = Arc::new(FilesystemStorage::new(&storage_path).unwrap()) as Arc<dyn BundleStorage>;
        let state = AppState::new(Arc::new(db), Config::default(), storage);

        let mut rx = state.subscribe_events();

        state.broadcast_event(ServerEvent::Ping {
            timestamp: chrono::Utc::now(),
        });

        let event = rx.recv().await.unwrap();
        assert!(matches!(event, ServerEvent::Ping { .. }));
    }
}
