//! Retention purge orchestration (Plan 04, step 6).
//!
//! Ties the governance records in the management DB (retention windows +
//! legal holds, `AuditGovernanceRepository`) to the ClickHouse purge
//! (`DecisionStore::purge_expired`). Used by both the manual
//! `POST /orgs/{org}/audit/purge` endpoint and the background sweeper that
//! replaces the static ClickHouse `TTL ... DELETE` (a static TTL would delete
//! held rows regardless — the exact failure legal holds exist to prevent).

use std::sync::Arc;

use super::{DecisionStore, DecisionStoreError, PurgeOutcome};
use crate::db::repositories::{AuditGovernanceRepository, OrganizationRepository};
use crate::db::{Database, DatabaseError};

/// Default retention window (days) for orgs without an explicit setting,
/// from `REAPER_AUDIT_DEFAULT_RETENTION_DAYS`. `0` disables default purging
/// (only orgs with an explicit retention setting are swept). The fallback of
/// 90 days matches the retention the ClickHouse schema's static TTL used to
/// enforce before purge moved to the application.
pub fn default_retention_days() -> i64 {
    std::env::var("REAPER_AUDIT_DEFAULT_RETENTION_DAYS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(90)
}

/// Sweeper interval (seconds) from `REAPER_AUDIT_PURGE_INTERVAL_SECS`.
/// `0` disables the background sweeper. Default: 6 hours.
pub fn purge_interval_secs() -> u64 {
    std::env::var("REAPER_AUDIT_PURGE_INTERVAL_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(6 * 3600)
}

/// RFC3339 cutoff for a retention window of `days`.
pub fn cutoff_for_days(days: i64) -> String {
    (chrono::Utc::now() - chrono::Duration::days(days)).to_rfc3339()
}

#[derive(Debug, thiserror::Error)]
pub enum PurgeError {
    #[error("retention is disabled (days must be > 0)")]
    RetentionDisabled,
    #[error(transparent)]
    Db(#[from] DatabaseError),
    #[error(transparent)]
    Store(#[from] DecisionStoreError),
}

/// Purge one org's expired decisions, honoring its active legal holds.
/// Checkpoints for the range are purged only when the org has **no** active
/// holds — checkpoints attest decision ranges, so while anything is held the
/// whole attestation chain is kept.
pub async fn run_org_purge(
    db: &Database,
    store: &DecisionStore,
    org_id: uuid::Uuid,
    days: i64,
) -> Result<PurgeOutcome, PurgeError> {
    if days <= 0 {
        return Err(PurgeError::RetentionDisabled);
    }
    let repo = AuditGovernanceRepository::new(db);
    let holds: Vec<_> = repo
        .active_holds(org_id)
        .await?
        .into_iter()
        .map(|h| h.filter)
        .collect();
    let cutoff = cutoff_for_days(days);
    let tenant = org_id.to_string();
    let outcome = store.purge_expired(&tenant, &cutoff, &holds).await?;
    if holds.is_empty() {
        store.purge_checkpoints(&tenant, &cutoff).await?;
    }
    Ok(outcome)
}

/// Spawn the background retention sweeper. No-op (with a log line) when the
/// decision store isn't configured or the interval is 0.
///
/// Multi-tenant (tenant-filtered) stores sweep per org: explicit retention
/// wins, otherwise the default window applies (default 0 ⇒ that org is
/// skipped). Single-tenant stores (`REAPER_CLICKHOUSE_TENANT_FILTER=false`)
/// run ONE global pass — per-org windows would race each other on unscoped
/// deletes (shortest window would win for everyone) — using the default
/// window and the union of every org's active holds.
pub fn spawn_retention_sweeper(state: Arc<crate::state::AppState>) {
    let Some(store) = state.decision_store.clone() else {
        return;
    };
    let interval_secs = purge_interval_secs();
    if interval_secs == 0 {
        tracing::info!("audit retention sweeper disabled (REAPER_AUDIT_PURGE_INTERVAL_SECS=0)");
        return;
    }
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // The first tick fires immediately; skip it so startup isn't a purge.
        ticker.tick().await;
        loop {
            ticker.tick().await;
            if let Err(e) = sweep_once(&state, &store).await {
                tracing::error!(error = %e, "audit retention sweep failed");
            }
        }
    });
    tracing::info!(
        interval_secs,
        default_retention_days = default_retention_days(),
        "audit retention sweeper started"
    );
}

/// One sweep pass over every tenant (or one global pass for single-tenant).
async fn sweep_once(
    state: &crate::state::AppState,
    store: &DecisionStore,
) -> Result<(), PurgeError> {
    let default_days = default_retention_days();
    let repo = AuditGovernanceRepository::new(&state.db);

    if !store.tenant_filter() {
        // Single-tenant: one global purge under the default window, honoring
        // every org's active holds (unscoped deletes see all rows).
        if default_days <= 0 {
            return Ok(());
        }
        let mut holds = Vec::new();
        let orgs = OrganizationRepository::new(&state.db)
            .list(None, None)
            .await?;
        for org in &orgs {
            holds.extend(
                repo.active_holds(org.id)
                    .await?
                    .into_iter()
                    .map(|h| h.filter),
            );
        }
        let cutoff = cutoff_for_days(default_days);
        let outcome = store.purge_expired("", &cutoff, &holds).await?;
        if holds.is_empty() {
            store.purge_checkpoints("", &cutoff).await?;
        }
        tracing::info!(?outcome, "audit retention sweep (single-tenant) done");
        return Ok(());
    }

    // Multi-tenant: per-org windows.
    let explicit: std::collections::HashMap<uuid::Uuid, i64> = repo
        .list_retention()
        .await?
        .into_iter()
        .map(|r| (r.org_id, r.days))
        .collect();
    let orgs = OrganizationRepository::new(&state.db)
        .list(None, None)
        .await?;
    let mut swept = 0usize;
    for org in orgs {
        let days = explicit.get(&org.id).copied().unwrap_or(default_days);
        if days <= 0 {
            continue; // retention disabled for this org
        }
        match run_org_purge(&state.db, store, org.id, days).await {
            Ok(_) => swept += 1,
            Err(e) => {
                // One org's failure must not starve the rest of the fleet.
                tracing::error!(org_id = %org.id, error = %e, "org retention purge failed");
            }
        }
    }
    tracing::info!(orgs_swept = swept, "audit retention sweep done");
    Ok(())
}
