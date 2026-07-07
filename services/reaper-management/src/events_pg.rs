//! PostgreSQL LISTEN/NOTIFY event bridge.
//!
//! With a single management instance, SSE fan-out is purely in-process:
//! publish → broadcast channel → connected agents. Run TWO OR MORE
//! instances behind a load balancer and that breaks silently — an agent
//! connected to instance B never hears about a publish that happened on
//! instance A. The delta-sync protocol self-heals on its polling interval,
//! but the instant wake-up is lost.
//!
//! On PostgreSQL we close that gap with the database's own eventing:
//! the publishing instance `pg_notify`s a JSON payload on `reaper_events`,
//! every instance LISTENs, and foreign notifications are re-broadcast into
//! the local SSE channel. Notifications tagged with our own instance id
//! are skipped (the local broadcast already happened, synchronously).
//!
//! LISTEN/NOTIFY is a wake-up hint, never a source of truth: replicas
//! still pull deltas by sequence number, so a dropped notification costs
//! only latency, not data. (On SQLite this module is inert — a single
//! file database implies a single instance.)

use std::sync::Arc;

use sqlx::postgres::PgListener;
use tracing::{info, warn};
use uuid::Uuid;

use crate::state::{AppState, ServerEvent};

/// Channel name shared by all management instances on one database.
pub const PG_EVENT_CHANNEL: &str = "reaper_events";

/// This process's identity, used to skip echoes of its own notifications.
fn instance_id() -> Uuid {
    static ID: std::sync::OnceLock<Uuid> = std::sync::OnceLock::new();
    *ID.get_or_init(Uuid::new_v4)
}

/// Publish a datastore-published event to sibling instances. No-op unless
/// the backing database is PostgreSQL. Failure is logged, not propagated:
/// the publish itself already committed and local subscribers were already
/// notified — a missed cross-instance wake-up only delays the next pull.
pub async fn notify_datastore_published(
    state: &AppState,
    datastore_id: Uuid,
    org_id: Uuid,
    namespace_id: Option<Uuid>,
    version: i64,
    checksum: &str,
) {
    if state.db.db_type() != "postgres" {
        return;
    }
    let Some(pool) = state.db.any_pool() else {
        return;
    };
    let payload = serde_json::json!({
        "instance": instance_id(),
        "type": "datastore_published",
        "datastore_id": datastore_id,
        "org_id": org_id,
        "namespace_id": namespace_id,
        "version": version,
        "checksum": checksum,
    })
    .to_string();
    if let Err(e) = sqlx::query("SELECT pg_notify($1, $2)")
        .bind(PG_EVENT_CHANNEL)
        .bind(&payload)
        .execute(pool)
        .await
    {
        warn!("pg_notify(datastore_published) failed: {e}");
    }
}

/// Spawn the LISTEN side: a background task that re-broadcasts foreign
/// instances' notifications into this instance's SSE channel. Call once at
/// startup when the database is PostgreSQL; `database_url` must be the
/// same URL the pool uses. `PgListener::recv` reconnects and re-LISTENs
/// automatically after connection loss.
pub fn spawn_pg_event_bridge(state: Arc<AppState>, database_url: String) {
    tokio::spawn(async move {
        let mut listener = match PgListener::connect(&database_url).await {
            Ok(l) => l,
            Err(e) => {
                warn!("pg event bridge: connect failed, cross-instance wake-ups disabled: {e}");
                return;
            }
        };
        if let Err(e) = listener.listen(PG_EVENT_CHANNEL).await {
            warn!("pg event bridge: LISTEN failed, cross-instance wake-ups disabled: {e}");
            return;
        }
        info!("pg event bridge: listening on '{}'", PG_EVENT_CHANNEL);

        loop {
            let notification = match listener.recv().await {
                Ok(n) => n,
                Err(e) => {
                    // recv() already retried its internal reconnect; back
                    // off briefly and let the next recv() re-establish.
                    warn!("pg event bridge: recv error (will retry): {e}");
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    continue;
                }
            };
            let Ok(payload) = serde_json::from_str::<serde_json::Value>(notification.payload())
            else {
                continue;
            };
            // Skip our own echo — local subscribers heard it synchronously.
            if payload["instance"].as_str() == Some(&instance_id().to_string()) {
                continue;
            }
            if payload["type"].as_str() != Some("datastore_published") {
                continue;
            }
            let (Some(datastore_id), Some(org_id)) = (
                payload["datastore_id"]
                    .as_str()
                    .and_then(|s| Uuid::parse_str(s).ok()),
                payload["org_id"]
                    .as_str()
                    .and_then(|s| Uuid::parse_str(s).ok()),
            ) else {
                continue;
            };
            let _ = state.event_tx.send(ServerEvent::DatastorePublished {
                datastore_id,
                org_id,
                namespace_id: payload["namespace_id"]
                    .as_str()
                    .and_then(|s| Uuid::parse_str(s).ok()),
                version: payload["version"].as_i64().unwrap_or(0),
                checksum: payload["checksum"].as_str().unwrap_or_default().to_string(),
            });
        }
    });
}
