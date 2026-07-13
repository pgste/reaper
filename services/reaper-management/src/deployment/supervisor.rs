//! Autonomous auto-rollback control loop — the rollout supervisor
//! (B2, closes PROD R2-1 / design T-3).
//!
//! Auto-rollback thresholds, the error-rate signal, the trigger evaluation,
//! and the rollback action all pre-existed; this module is the supervising
//! loop that ties them together so a bad rollout self-reverts without a
//! human polling `check-rollback`.
//!
//! Each tick the supervisor:
//! 1. elects a leader across replicas via a per-tick advisory try-lock
//!    (same pattern as the change-log sweeper; SQLite dev runs are
//!    single-process and skip election);
//! 2. enumerates every ACTIVE rollout across all orgs (one indexed query);
//! 3. evaluates each against its resolved auto-rollback config through
//!    `DeploymentService::evaluate_rollback_trigger` — the same code path
//!    the operator-facing endpoints use;
//! 4. when the trigger fires:
//!    * `monitor` mode (the safe default): audit + SSE event + warn log +
//!      Prometheus counter, but NO action. Each rollout is flagged at most
//!      once per supervisor lifetime (in-memory set; a restart re-alerts
//!      once, which is acceptable).
//!    * `enforce` mode: cancel the triggering rollout, then start a rollback
//!      to the previous bundle. The rollback rollout is stamped with
//!      [`AUTO_ROLLBACK_TRIGGER`] in `rollouts.triggered_by`, and the
//!      supervisor skips any rollout carrying that marker — the loop guard
//!      that keeps remediation from being re-rolled-back.
//!
//! Failures handling one rollout never kill the loop: they are logged,
//! counted in the pass report, and the pass continues.
//!
//! Configuration:
//! * `REAPER_ROLLOUT_SUPERVISOR_ENABLED` — default `true`
//! * `REAPER_ROLLOUT_SUPERVISOR_INTERVAL_SECS` — default 30, minimum 5

use std::collections::HashSet;
use std::sync::Arc;

use tracing::{info, warn};
use uuid::Uuid;

use crate::audit::{actions, ActorType, AuditEntry, ResourceType};
use crate::db::{advisory_keys, AdvisoryLock};
use crate::domain::agent_deployment::RollbackMode;
use crate::domain::deployment::Rollout;
use crate::state::{AppState, ServerEvent};

use super::{DeploymentError, DeploymentService};

/// Provenance marker stamped into `rollouts.triggered_by` on rollback
/// rollouts the supervisor starts. Rollouts carrying it are excluded from
/// supervision so the supervisor never rolls back its own remediation.
pub const AUTO_ROLLBACK_TRIGGER: &str = "auto_rollback";

/// Actor id recorded on supervisor-authored audit entries (actor type
/// `system`).
const SUPERVISOR_ACTOR: &str = "rollout-supervisor";

/// What one supervisor pass did — returned so tests (and logs) can assert a
/// single deterministic tick without spawning the loop.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SupervisorPassReport {
    /// Active rollouts evaluated (supervisor-created rollbacks excluded)
    pub evaluated: usize,
    /// Rollouts whose trigger fired this pass (monitor alerts + enforcements)
    pub triggered: usize,
    /// Enforce-mode rollbacks actually started
    pub enforced: usize,
    /// Rollouts skipped because they were already flagged in monitor mode
    pub already_flagged: usize,
    /// Per-rollout failures (logged and skipped, never fatal to the pass)
    pub errors: usize,
}

/// Run exactly one supervisor tick over all active rollouts.
///
/// `flagged` carries monitor-mode alert state between ticks so a breaching
/// rollout is audited/alerted once, not every interval; entries for rollouts
/// that leave the active set are dropped to keep it bounded.
pub async fn run_supervisor_pass(
    state: &AppState,
    flagged: &mut HashSet<Uuid>,
) -> SupervisorPassReport {
    let mut report = SupervisorPassReport::default();
    let service = DeploymentService::new(state.db.clone());

    let active = match service.list_active_rollouts_global().await {
        Ok(active) => active,
        Err(e) => {
            warn!(error = %e, "rollout supervisor: failed to list active rollouts");
            report.errors += 1;
            return report;
        }
    };

    // Drop flags for rollouts that are no longer active (completed,
    // cancelled, …) so the in-memory set stays bounded.
    let active_ids: HashSet<Uuid> = active.iter().map(|(r, _)| r.id).collect();
    flagged.retain(|id| active_ids.contains(id));

    for (rollout, org_id) in &active {
        // Loop guard: never evaluate remediation the supervisor started.
        if rollout.triggered_by.as_deref() == Some(AUTO_ROLLBACK_TRIGGER) {
            continue;
        }
        report.evaluated += 1;

        match supervise_rollout(state, &service, *org_id, rollout, flagged).await {
            Ok(RolloutOutcome::NotTriggered) => {}
            Ok(RolloutOutcome::AlreadyFlagged) => report.already_flagged += 1,
            Ok(RolloutOutcome::Alerted) => report.triggered += 1,
            Ok(RolloutOutcome::Enforced) => {
                report.triggered += 1;
                report.enforced += 1;
            }
            // One rollout's failure must not kill the pass — log + continue.
            Err(e) => {
                report.errors += 1;
                warn!(
                    rollout_id = %rollout.id,
                    org_id = %org_id,
                    error = %e,
                    "rollout supervisor: supervision failed for rollout; continuing"
                );
            }
        }
    }

    report
}

enum RolloutOutcome {
    NotTriggered,
    AlreadyFlagged,
    Alerted,
    Enforced,
}

async fn supervise_rollout(
    state: &AppState,
    service: &DeploymentService,
    org_id: Uuid,
    rollout: &Rollout,
    flagged: &mut HashSet<Uuid>,
) -> Result<RolloutOutcome, DeploymentError> {
    let eval = service.evaluate_rollback_trigger(org_id, rollout).await?;
    if !eval.should_rollback {
        return Ok(RolloutOutcome::NotTriggered);
    }

    match eval.mode {
        RollbackMode::Monitor => {
            // Alert-only dry run: flag once per active rollout to avoid
            // spamming audit/SSE every tick.
            if !flagged.insert(rollout.id) {
                return Ok(RolloutOutcome::AlreadyFlagged);
            }

            warn!(
                rollout_id = %rollout.id,
                org_id = %org_id,
                error_rate = eval.current_error_rate,
                threshold = eval.threshold,
                "auto-rollback trigger fired in MONITOR mode: taking no action \
                 (arm mode=enforce to let the supervisor roll back)"
            );
            crate::metrics::AUTO_ROLLBACKS_TOTAL
                .with_label_values(&[org_id.to_string().as_str(), "monitor"])
                .inc();

            AuditEntry::builder(
                actions::DEPLOYMENT_AUTO_ROLLBACK_TRIGGERED,
                ActorType::System,
                SUPERVISOR_ACTOR,
            )
            .org_id(org_id)
            .resource(ResourceType::Rollout, rollout.id.to_string())
            .details(serde_json::json!({
                "mode": "monitor",
                "error_rate": eval.current_error_rate,
                "threshold": eval.threshold,
                "completed_count": eval.completed_count,
                "reason": eval.reason,
            }))
            .log(&state.db)
            .await
            .map_err(|e| DeploymentError::InvalidState(format!("audit write failed: {e}")))?;

            state.broadcast_event(ServerEvent::AutoRollbackTriggered {
                rollout_id: rollout.id,
                org_id,
                namespace_id: rollout.namespace_id,
                error_rate: eval.current_error_rate,
                threshold: eval.threshold,
                enforced: false,
                rollback_rollout_id: None,
            });

            Ok(RolloutOutcome::Alerted)
        }

        RollbackMode::Enforce => {
            let reason = format!(
                "auto-rollback by rollout supervisor: error rate {:.2}% exceeded threshold {:.2}%",
                eval.current_error_rate, eval.threshold
            );

            // Stop the bleeding first: cancel the triggering rollout …
            service.cancel_rollout(rollout.id, &reason, state).await?;

            // … then restore the previous bundle. The rollback rollout is
            // stamped AUTO_ROLLBACK_TRIGGER so later ticks skip it.
            let result = service
                .rollback(
                    org_id,
                    rollout.namespace_id,
                    None, // previous bundle
                    &reason,
                    Some(AUTO_ROLLBACK_TRIGGER),
                    state,
                )
                .await?;

            warn!(
                rollout_id = %rollout.id,
                org_id = %org_id,
                rollback_rollout_id = %result.rollout.id,
                error_rate = eval.current_error_rate,
                threshold = eval.threshold,
                "auto-rollback ENFORCED: rollout cancelled and rollback started"
            );
            crate::metrics::AUTO_ROLLBACKS_TOTAL
                .with_label_values(&[org_id.to_string().as_str(), "enforce"])
                .inc();

            AuditEntry::builder(
                actions::DEPLOYMENT_AUTO_ROLLBACK,
                ActorType::System,
                SUPERVISOR_ACTOR,
            )
            .org_id(org_id)
            .resource(ResourceType::Rollout, rollout.id.to_string())
            .details(serde_json::json!({
                "mode": "enforce",
                "error_rate": eval.current_error_rate,
                "threshold": eval.threshold,
                "completed_count": eval.completed_count,
                "reason": eval.reason,
                "rollback_rollout_id": result.rollout.id,
                "rollback_bundle_id": result.rollout.bundle_id,
            }))
            .log(&state.db)
            .await
            .map_err(|e| DeploymentError::InvalidState(format!("audit write failed: {e}")))?;

            state.broadcast_event(ServerEvent::AutoRollbackTriggered {
                rollout_id: rollout.id,
                org_id,
                namespace_id: rollout.namespace_id,
                error_rate: eval.current_error_rate,
                threshold: eval.threshold,
                enforced: true,
                rollback_rollout_id: Some(result.rollout.id),
            });

            Ok(RolloutOutcome::Enforced)
        }
    }
}

/// Spawn the rollout supervisor background loop (wired from `main.rs` like
/// the other background tasks). No-op when
/// `REAPER_ROLLOUT_SUPERVISOR_ENABLED=false`.
pub fn spawn_rollout_supervisor(state: Arc<AppState>) {
    let enabled = std::env::var("REAPER_ROLLOUT_SUPERVISOR_ENABLED")
        .map(|v| !matches!(v.to_lowercase().as_str(), "false" | "0" | "no" | "off"))
        .unwrap_or(true);
    if !enabled {
        info!("rollout supervisor disabled (REAPER_ROLLOUT_SUPERVISOR_ENABLED=false)");
        return;
    }

    let interval_secs: u64 = std::env::var("REAPER_ROLLOUT_SUPERVISOR_INTERVAL_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30)
        .max(5);

    info!(
        interval_secs,
        "rollout supervisor enabled (autonomous auto-rollback; per-namespace \
         mode monitor|enforce via the auto-rollback config)"
    );

    tokio::spawn(async move {
        // Monitor-mode alert de-duplication across ticks. In-memory on
        // purpose: a restart (or leadership change) re-alerts each still-
        // breaching rollout once, which is the desired failure behavior.
        let mut flagged: HashSet<Uuid> = HashSet::new();

        let mut tick = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;

            // Under N replicas only one instance supervises per tick: a
            // per-tick advisory try-lock elects it (same pattern as the
            // change-log sweeper). The lock releases with the guard
            // transaction at the end of the iteration.
            let _leader_lock = match state
                .db
                .try_advisory_xact_lock(advisory_keys::ROLLOUT_SUPERVISOR)
                .await
            {
                Ok(AdvisoryLock::Acquired(tx)) => Some(tx),
                Ok(AdvisoryLock::Unsupported) => None, // sqlite: single process
                Ok(AdvisoryLock::Busy) => continue,    // sibling replica has this tick
                Err(e) => {
                    warn!(error = %e, "rollout supervisor leader election failed");
                    continue;
                }
            };

            let report = run_supervisor_pass(&state, &mut flagged).await;
            if report.triggered > 0 || report.errors > 0 {
                info!(
                    evaluated = report.evaluated,
                    triggered = report.triggered,
                    enforced = report.enforced,
                    errors = report.errors,
                    "rollout supervisor pass complete"
                );
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::{AuditQuery, AuditRepository};
    use crate::db::repositories::{AgentDeploymentRepository, RollbackConfigRepository};
    use crate::db::Database;
    use crate::domain::agent_deployment::{AgentDeploymentStatus, RollbackConfig};
    use crate::domain::deployment::{RolloutStatus, StartRollout};
    use crate::storage::FilesystemStorage;
    use tempfile::TempDir;

    async fn setup() -> (TempDir, Arc<Database>, AppState) {
        let temp_dir = TempDir::new().unwrap();
        let storage_path = temp_dir.path().join("storage");
        std::fs::create_dir_all(&storage_path).unwrap();

        let db_config = crate::db::ephemeral_test_config(temp_dir.path()).await;
        let db = Database::new(&db_config).await.unwrap();
        db.run_migrations().await.unwrap();
        let db = Arc::new(db);

        let storage = Arc::new(FilesystemStorage::new(&storage_path).unwrap())
            as Arc<dyn crate::storage::BundleStorage>;
        let state = AppState::new(db.clone(), crate::config::Config::default(), storage);

        (temp_dir, db, state)
    }

    async fn create_test_org(db: &Database) -> Uuid {
        let pool = db.any_pool().unwrap();
        let org_id = Uuid::new_v4();
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO organizations (id, name, slug, created_at, updated_at) VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(org_id.to_string())
        .bind("Supervisor Org")
        .bind(format!("supervisor-org-{}", org_id))
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();
        org_id
    }

    async fn create_test_bundle(db: &Database, org_id: Uuid, name: &str, status: &str) -> Uuid {
        let pool = db.any_pool().unwrap();
        let bundle_id = Uuid::new_v4();
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO bundles (id, org_id, name, version, status, policy_count, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(bundle_id.to_string())
        .bind(org_id.to_string())
        .bind(name)
        .bind("1.0.0")
        .bind(status)
        .bind(0)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();
        bundle_id
    }

    async fn create_test_agents(db: &Database, org_id: Uuid, count: usize) -> Vec<Uuid> {
        let pool = db.any_pool().unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        let mut agent_ids = Vec::new();
        for i in 0..count {
            let agent_id = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO agents (id, org_id, name, status, registered_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6)",
            )
            .bind(agent_id.to_string())
            .bind(org_id.to_string())
            .bind(format!("sup-agent-{}", i))
            .bind("active")
            .bind(&now)
            .bind(&now)
            .execute(pool)
            .await
            .unwrap();
            agent_ids.push(agent_id);
        }
        agent_ids
    }

    async fn upsert_config(db: &Database, org_id: Uuid, mode: RollbackMode) {
        let mut config = RollbackConfig::new(org_id, None);
        config.is_enabled = true;
        config.error_rate_threshold = 50.0;
        config.min_requests = 1;
        config.mode = mode;
        RollbackConfigRepository::new(db)
            .upsert(&config)
            .await
            .unwrap();
    }

    /// Start a rollout to `bundle_id` and mark every agent deployment failed.
    async fn failing_rollout(
        db: &Arc<Database>,
        org_id: Uuid,
        bundle_id: Uuid,
        state: &AppState,
    ) -> Uuid {
        let service = DeploymentService::new(db.clone());
        let result = service
            .start_rollout(
                org_id,
                &StartRollout {
                    bundle_id,
                    strategy_id: None,
                    namespace_id: None,
                    triggered_by: None,
                },
                state,
            )
            .await
            .unwrap();

        let dep_repo = AgentDeploymentRepository::new(db);
        for dep in dep_repo.get_by_rollout(result.rollout.id).await.unwrap() {
            dep_repo
                .update_status(dep.id, AgentDeploymentStatus::Failed, Some("boom"))
                .await
                .unwrap();
        }
        result.rollout.id
    }

    async fn audit_count(db: &Database, action: &str) -> u64 {
        AuditRepository::new(db)
            .count(&AuditQuery {
                action: Some(action.to_string()),
                ..Default::default()
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn enforce_mode_cancels_rollout_and_starts_marked_rollback() {
        let (_tmp, db, state) = setup().await;
        let org_id = create_test_org(&db).await;
        // The rollback target: promotion demoted the previous bundle to
        // Deprecated — exactly what the supervisor should restore.
        let previous = create_test_bundle(&db, org_id, "previous", "deprecated").await;
        let bad = create_test_bundle(&db, org_id, "bad", "compiled").await;
        let _agents = create_test_agents(&db, org_id, 3).await;
        upsert_config(&db, org_id, RollbackMode::Enforce).await;

        let rollout_id = failing_rollout(&db, org_id, bad, &state).await;

        let mut flagged = HashSet::new();
        let report = run_supervisor_pass(&state, &mut flagged).await;
        assert_eq!(report.evaluated, 1);
        assert_eq!(report.triggered, 1);
        assert_eq!(report.enforced, 1);
        assert_eq!(report.errors, 0);

        // The bad rollout is cancelled with the supervisor's reason.
        let service = DeploymentService::new(db.clone());
        let rollout = service.get_rollout(rollout_id).await.unwrap();
        assert_eq!(rollout.status, RolloutStatus::Cancelled);
        assert!(
            rollout
                .error
                .as_deref()
                .unwrap_or("")
                .contains("auto-rollback"),
            "cancel reason names the supervisor: {:?}",
            rollout.error
        );

        // A rollback rollout to the previous bundle exists, stamped with the
        // loop-guard marker.
        let rollouts = service.list_rollouts(org_id, None, 50).await.unwrap();
        let rollback = rollouts
            .iter()
            .find(|r| r.triggered_by.as_deref() == Some(AUTO_ROLLBACK_TRIGGER))
            .expect("supervisor-started rollback rollout exists");
        assert_eq!(rollback.bundle_id, previous);
        assert!(!rollback.is_terminal());

        // The action is audited as the system actor.
        assert_eq!(audit_count(&db, actions::DEPLOYMENT_AUTO_ROLLBACK).await, 1);

        // Loop guard: the next pass sees only the supervisor's own rollback
        // rollout, skips it, and does nothing more.
        let report = run_supervisor_pass(&state, &mut flagged).await;
        assert_eq!(report.evaluated, 0);
        assert_eq!(report.enforced, 0);
        assert_eq!(audit_count(&db, actions::DEPLOYMENT_AUTO_ROLLBACK).await, 1);
    }

    #[tokio::test]
    async fn monitor_mode_audits_once_and_takes_no_action() {
        let (_tmp, db, state) = setup().await;
        let org_id = create_test_org(&db).await;
        let _previous = create_test_bundle(&db, org_id, "previous", "deprecated").await;
        let bad = create_test_bundle(&db, org_id, "bad", "compiled").await;
        let _agents = create_test_agents(&db, org_id, 3).await;
        upsert_config(&db, org_id, RollbackMode::Monitor).await;

        let rollout_id = failing_rollout(&db, org_id, bad, &state).await;

        let mut flagged = HashSet::new();
        let report = run_supervisor_pass(&state, &mut flagged).await;
        assert_eq!(report.triggered, 1);
        assert_eq!(report.enforced, 0);

        // No action: the rollout is untouched and no rollback exists.
        let service = DeploymentService::new(db.clone());
        let rollout = service.get_rollout(rollout_id).await.unwrap();
        assert_ne!(rollout.status, RolloutStatus::Cancelled);
        let rollouts = service.list_rollouts(org_id, None, 50).await.unwrap();
        assert_eq!(rollouts.len(), 1, "no rollback rollout was started");

        // Audited exactly once …
        assert_eq!(
            audit_count(&db, actions::DEPLOYMENT_AUTO_ROLLBACK_TRIGGERED).await,
            1
        );

        // … and NOT re-audited on the next tick (alert de-dup).
        let report = run_supervisor_pass(&state, &mut flagged).await;
        assert_eq!(report.triggered, 0);
        assert_eq!(report.already_flagged, 1);
        assert_eq!(
            audit_count(&db, actions::DEPLOYMENT_AUTO_ROLLBACK_TRIGGERED).await,
            1
        );
    }

    #[tokio::test]
    async fn below_threshold_or_disabled_takes_no_action() {
        let (_tmp, db, state) = setup().await;
        let org_id = create_test_org(&db).await;
        let bundle = create_test_bundle(&db, org_id, "good", "compiled").await;
        let agents = create_test_agents(&db, org_id, 3).await;

        // No config at all (disabled default): nothing fires.
        let service = DeploymentService::new(db.clone());
        let result = service
            .start_rollout(
                org_id,
                &StartRollout {
                    bundle_id: bundle,
                    strategy_id: None,
                    namespace_id: None,
                    triggered_by: None,
                },
                &state,
            )
            .await
            .unwrap();
        let rollout_id = result.rollout.id;

        let mut flagged = HashSet::new();
        let report = run_supervisor_pass(&state, &mut flagged).await;
        assert_eq!(report.evaluated, 1);
        assert_eq!(report.triggered, 0);

        // Enabled + enforce, but the fleet is healthy (all deployed): still
        // nothing fires.
        upsert_config(&db, org_id, RollbackMode::Enforce).await;
        let dep_repo = AgentDeploymentRepository::new(&db);
        for agent in &agents {
            let dep = dep_repo
                .get_latest_for_agent_bundle(*agent, bundle)
                .await
                .unwrap()
                .unwrap();
            dep_repo
                .update_status(dep.id, AgentDeploymentStatus::Deployed, None)
                .await
                .unwrap();
        }

        let report = run_supervisor_pass(&state, &mut flagged).await;
        assert_eq!(report.evaluated, 1);
        assert_eq!(report.triggered, 0);
        assert_eq!(report.errors, 0);

        let rollout = service.get_rollout(rollout_id).await.unwrap();
        assert_ne!(rollout.status, RolloutStatus::Cancelled);
        assert_eq!(audit_count(&db, actions::DEPLOYMENT_AUTO_ROLLBACK).await, 0);
        assert_eq!(
            audit_count(&db, actions::DEPLOYMENT_AUTO_ROLLBACK_TRIGGERED).await,
            0
        );
    }
}
