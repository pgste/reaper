//! Deployment service for managing rollouts
//!
//! Orchestrates policy deployments using various strategies including
//! immediate, canary, percentage-based, and label-selector rollouts.

mod helpers;
pub mod types;

use std::sync::Arc;
use tracing::info;
use uuid::Uuid;

use chrono::Utc;

use crate::db::repositories::{
    AgentDeploymentRepository, AgentRepository, BundleRepository, DeploymentRepository,
    RollbackConfigRepository,
};
use crate::db::Database;
use crate::domain::agent::AgentStatus;
use crate::domain::agent_deployment::{AgentDeployment, AgentDeploymentStatus, RollbackConfig};
use crate::domain::bundle::BundleStatus;
use crate::domain::deployment::{
    CreateDeploymentStrategy, CreateVersionPin, DeploymentStrategy, Rollout, RolloutStatus,
    RolloutWave, StartRollout, StrategyConfig, StrategyType, VersionPin, WaveStatus,
};
use crate::state::{AppState, ServerEvent};

pub use types::{
    AgentInfo, DeploymentError, DryRunResult, RollbackTriggerEvaluation, RolloutResult,
    SkippedAgent, StrategyInfo,
};

/// Whether rollout waves wait for agents to confirm their applied version
/// before completing (default true). Set `REAPER_REQUIRE_AGENT_CONFIRMATION=false`
/// to keep the old optimistic behavior (useful for fleets whose agents don't
/// report yet).
fn confirmation_required() -> bool {
    static V: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *V.get_or_init(|| {
        std::env::var("REAPER_REQUIRE_AGENT_CONFIRMATION")
            .map(|v| !matches!(v.to_lowercase().as_str(), "false" | "0" | "no" | "off"))
            .unwrap_or(true)
    })
}

/// Seconds after which an unconfirmed (still-pending) agent deployment is
/// treated as settled (timed out) for wave-completion purposes, so a crashed or
/// non-reporting agent cannot wedge a rollout forever.
fn confirmation_timeout_secs() -> i64 {
    static V: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
    *V.get_or_init(|| {
        std::env::var("REAPER_DEPLOY_CONFIRMATION_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(300)
    })
}

/// Service for managing deployments and rollouts
pub struct DeploymentService {
    pub(super) db: Arc<Database>,
    /// When true, waves complete only after agents confirm their applied
    /// version (see `record_agent_report`); when false, waves complete
    /// optimistically as soon as the bundle-promoted event is broadcast.
    pub(super) require_agent_confirmation: bool,
}

impl DeploymentService {
    /// Create a new deployment service
    pub fn new(db: Arc<Database>) -> Self {
        Self {
            db,
            require_agent_confirmation: confirmation_required(),
        }
    }

    /// Record an agent's report of its applied bundle version, updating that
    /// agent's deployment record (fail-closed truth) and, when confirmation is
    /// required and the deployment belongs to a rollout, advancing rollout/wave
    /// completion based on real confirmations rather than optimism.
    pub async fn record_agent_report(
        &self,
        agent_id: Uuid,
        bundle_id: Uuid,
        status: AgentDeploymentStatus,
        error: Option<String>,
        state: &AppState,
    ) -> Result<(), DeploymentError> {
        let dep_repo = AgentDeploymentRepository::new(&self.db);

        let rollout_id = match dep_repo
            .get_latest_for_agent_bundle(agent_id, bundle_id)
            .await?
        {
            Some(dep) => {
                if !dep.is_terminal() {
                    dep_repo
                        .update_status(dep.id, status, error.as_deref())
                        .await?;
                    dep_repo.acknowledge(dep.id).await?;
                }
                dep.rollout_id
            }
            None => {
                // Report for a deployment we weren't tracking (e.g. a direct
                // promote outside a rollout) — still record the actual state.
                let mut dep = AgentDeployment::new(agent_id, bundle_id, None);
                match status {
                    AgentDeploymentStatus::Deployed => dep.mark_deployed(),
                    AgentDeploymentStatus::Failed => {
                        dep.mark_failed(error.clone().unwrap_or_default())
                    }
                    _ => {}
                }
                dep.acknowledge();
                dep_repo.create(&dep).await?;
                None
            }
        };

        info!(agent_id = %agent_id, bundle_id = %bundle_id, status = %status,
            "Recorded agent deployment report");

        if self.require_agent_confirmation {
            if let Some(rollout_id) = rollout_id {
                self.try_advance_on_confirmation(rollout_id, state).await?;
            }
        }
        Ok(())
    }

    /// Complete any wave whose target agents have all settled (confirmed
    /// terminal or timed out), and complete the rollout when all waves are done.
    /// Does not auto-advance to the next wave — multi-wave progression stays with
    /// the existing approval flow.
    async fn try_advance_on_confirmation(
        &self,
        rollout_id: Uuid,
        state: &AppState,
    ) -> Result<(), DeploymentError> {
        let repo = DeploymentRepository::new(&self.db);
        let dep_repo = AgentDeploymentRepository::new(&self.db);

        let Some(rollout) = repo.get_rollout_by_id(rollout_id).await? else {
            return Ok(());
        };
        if rollout.status == RolloutStatus::Completed {
            return Ok(());
        }

        let waves = repo.get_waves_for_rollout(rollout_id).await?;
        let deps = dep_repo.get_by_rollout(rollout_id).await?;
        let timeout = confirmation_timeout_secs();
        let now = Utc::now();

        // An agent is "settled" for completion once its latest deployment for
        // this rollout is terminal, or has been pending past the timeout.
        let settled = |agent_id: &Uuid| -> bool {
            deps.iter()
                .filter(|d| d.agent_id == *agent_id)
                .max_by_key(|d| d.created_at)
                .map(|d| d.is_terminal() || (now - d.created_at).num_seconds() >= timeout)
                .unwrap_or(false)
        };

        for wave in waves.iter().filter(|w| w.status == WaveStatus::Deploying) {
            if !wave.target_agents.is_empty() && wave.target_agents.iter().all(&settled) {
                repo.update_wave_status(wave.id, WaveStatus::Completed)
                    .await?;
                repo.increment_deployed_count(rollout_id, wave.target_agents.len() as u32)
                    .await?;
                info!(rollout_id = %rollout_id, wave = wave.wave_number,
                    "Wave confirmed complete by agents");
            }
        }

        let waves = repo.get_waves_for_rollout(rollout_id).await?;
        if !waves.is_empty() && waves.iter().all(|w| w.status == WaveStatus::Completed) {
            repo.update_rollout_status(rollout_id, RolloutStatus::Completed, None)
                .await?;
            let any_failed = deps
                .iter()
                .any(|d| d.status == AgentDeploymentStatus::Failed);
            if let Some(bundle) = BundleRepository::new(&self.db)
                .get_by_id(rollout.bundle_id)
                .await?
            {
                state.broadcast_event(ServerEvent::RolloutCompleted {
                    rollout_id,
                    bundle_id: rollout.bundle_id,
                    org_id: bundle.org_id,
                    namespace_id: rollout.namespace_id,
                    success: !any_failed,
                });
            }
            info!(rollout_id = %rollout_id, success = !any_failed,
                "Rollout completed (agent-confirmed)");
        }
        Ok(())
    }

    // ==================== Strategy Operations ====================

    /// Create a new deployment strategy
    pub async fn create_strategy(
        &self,
        org_id: Uuid,
        input: &CreateDeploymentStrategy,
    ) -> Result<DeploymentStrategy, DeploymentError> {
        let repo = DeploymentRepository::new(&self.db);
        let strategy = repo.create_strategy(org_id, input).await?;
        info!(
            strategy_id = %strategy.id,
            name = %strategy.name,
            strategy_type = %strategy.strategy_type,
            "Deployment strategy created"
        );
        Ok(strategy)
    }

    /// Get a deployment strategy by ID
    pub async fn get_strategy(&self, id: Uuid) -> Result<DeploymentStrategy, DeploymentError> {
        let repo = DeploymentRepository::new(&self.db);
        repo.get_strategy_by_id(id)
            .await?
            .ok_or_else(|| DeploymentError::StrategyNotFound(id.to_string()))
    }

    /// List deployment strategies
    pub async fn list_strategies(
        &self,
        org_id: Uuid,
        namespace_id: Option<Uuid>,
    ) -> Result<Vec<DeploymentStrategy>, DeploymentError> {
        let repo = DeploymentRepository::new(&self.db);
        Ok(repo.list_strategies(org_id, namespace_id).await?)
    }

    /// Delete a deployment strategy
    pub async fn delete_strategy(&self, id: Uuid) -> Result<(), DeploymentError> {
        let repo = DeploymentRepository::new(&self.db);
        repo.delete_strategy(id).await?;
        info!(strategy_id = %id, "Deployment strategy deleted");
        Ok(())
    }

    // ==================== Rollout Operations ====================

    /// Dry-run a rollout to preview what would happen
    pub async fn dry_run_rollout(
        &self,
        org_id: Uuid,
        bundle_id: Uuid,
        strategy_id: Option<Uuid>,
        namespace_id: Option<Uuid>,
    ) -> Result<DryRunResult, DeploymentError> {
        let bundle_repo = BundleRepository::new(&self.db);
        let deploy_repo = DeploymentRepository::new(&self.db);
        let agent_repo = AgentRepository::new(&self.db);

        let mut validation_errors = Vec::new();

        // Check bundle exists and is ready
        let bundle = bundle_repo
            .get_by_id(bundle_id)
            .await?
            .ok_or_else(|| DeploymentError::BundleNotFound(bundle_id.to_string()))?;

        if !matches!(
            bundle.status,
            BundleStatus::Compiled | BundleStatus::Staged | BundleStatus::Promoted
        ) {
            validation_errors.push(format!(
                "Bundle status is {:?}, must be Compiled, Staged, or Promoted",
                bundle.status
            ));
        }

        // Check for existing active rollouts
        let active_rollouts = deploy_repo
            .get_active_rollouts_for_bundle(bundle_id)
            .await?;
        if !active_rollouts.is_empty() {
            validation_errors.push(format!(
                "Active rollout already exists: {}",
                active_rollouts[0].id
            ));
        }

        // Get strategy
        let strategy = if let Some(strategy_id) = strategy_id {
            deploy_repo
                .get_strategy_by_id(strategy_id)
                .await?
                .map(|s| StrategyInfo {
                    id: s.id,
                    name: s.name,
                    strategy_type: s.strategy_type.to_string(),
                })
        } else {
            deploy_repo
                .get_default_strategy(org_id, namespace_id)
                .await?
                .map(|s| StrategyInfo {
                    id: s.id,
                    name: s.name,
                    strategy_type: s.strategy_type.to_string(),
                })
                .or_else(|| {
                    Some(StrategyInfo {
                        id: Uuid::nil(),
                        name: "immediate".to_string(),
                        strategy_type: "Immediate".to_string(),
                    })
                })
        };

        // Get all agents
        let all_agents = agent_repo.list_by_org(org_id).await?;

        // Separate active and inactive agents
        let mut would_deploy_to = Vec::new();
        let mut agents_skipped = Vec::new();

        for agent in all_agents {
            if agent.status == AgentStatus::Active {
                // Check for version pin
                let pin = deploy_repo.get_active_pin(agent.id).await?;
                if let Some(pin) = pin {
                    if pin.bundle_id != bundle_id {
                        agents_skipped.push(SkippedAgent {
                            id: agent.id,
                            name: agent.name.clone(),
                            reason: format!("Version pinned to bundle {}", pin.bundle_id),
                        });
                        continue;
                    }
                }

                would_deploy_to.push(AgentInfo {
                    id: agent.id,
                    name: agent.name,
                    hostname: agent.hostname,
                });
            } else {
                agents_skipped.push(SkippedAgent {
                    id: agent.id,
                    name: agent.name,
                    reason: format!("Agent status: {:?}", agent.status),
                });
            }
        }

        let target_count = would_deploy_to.len() as u32;

        if target_count == 0 && validation_errors.is_empty() {
            validation_errors.push("No active agents available for deployment".to_string());
        }

        Ok(DryRunResult {
            would_deploy_to,
            agents_skipped,
            target_count,
            validation_errors,
            strategy,
        })
    }

    /// Start a new rollout
    pub async fn start_rollout(
        &self,
        org_id: Uuid,
        input: &StartRollout,
        state: &AppState,
    ) -> Result<RolloutResult, DeploymentError> {
        self.start_rollout_with_options(org_id, input, state, false)
            .await
    }

    /// Start a rollout, optionally accepting a Deprecated bundle as the
    /// target. Only the rollback path sets `allow_deprecated_bundle`: the
    /// previous known-good bundle is Deprecated by the promotion flow, and
    /// restoring it is exactly what a rollback is. Direct rollouts keep
    /// rejecting Deprecated bundles.
    async fn start_rollout_with_options(
        &self,
        org_id: Uuid,
        input: &StartRollout,
        state: &AppState,
        allow_deprecated_bundle: bool,
    ) -> Result<RolloutResult, DeploymentError> {
        let bundle_repo = BundleRepository::new(&self.db);
        let deploy_repo = DeploymentRepository::new(&self.db);
        let agent_repo = AgentRepository::new(&self.db);

        // Verify bundle exists and is ready
        let bundle = bundle_repo
            .get_by_id(input.bundle_id)
            .await?
            .ok_or_else(|| DeploymentError::BundleNotFound(input.bundle_id.to_string()))?;

        let deployable = matches!(
            bundle.status,
            BundleStatus::Compiled | BundleStatus::Staged | BundleStatus::Promoted
        ) || (allow_deprecated_bundle
            && bundle.status == BundleStatus::Deprecated);
        if !deployable {
            return Err(DeploymentError::BundleNotReady(format!(
                "Bundle status is {:?}, must be Compiled, Staged, or Promoted",
                bundle.status
            )));
        }

        // Check for existing active rollouts
        let active_rollouts = deploy_repo
            .get_active_rollouts_for_bundle(input.bundle_id)
            .await?;
        if !active_rollouts.is_empty() {
            return Err(DeploymentError::ActiveRolloutExists(
                input.bundle_id.to_string(),
            ));
        }

        // Get or determine strategy
        let strategy = if let Some(strategy_id) = input.strategy_id {
            deploy_repo
                .get_strategy_by_id(strategy_id)
                .await?
                .ok_or_else(|| DeploymentError::StrategyNotFound(strategy_id.to_string()))?
        } else {
            // Get default strategy or create an immediate one
            deploy_repo
                .get_default_strategy(org_id, input.namespace_id)
                .await?
                .unwrap_or_else(|| DeploymentStrategy {
                    id: Uuid::nil(),
                    org_id,
                    namespace_id: input.namespace_id,
                    name: "immediate".to_string(),
                    strategy_type: StrategyType::Immediate,
                    config: StrategyConfig::Immediate {},
                    is_default: false,
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                })
        };

        // Get target agents
        let all_agents = agent_repo.list_by_org(org_id).await?;
        let active_agents: Vec<_> = all_agents
            .into_iter()
            .filter(|a| a.status == AgentStatus::Active)
            .collect();

        if active_agents.is_empty() {
            return Err(DeploymentError::NoAgentsAvailable);
        }

        // Filter agents based on strategy
        let target_agents = self.select_agents_for_strategy(&strategy, &active_agents);

        if target_agents.is_empty() {
            return Err(DeploymentError::NoAgentsAvailable);
        }

        // Create the rollout
        let rollout = deploy_repo
            .create_rollout(input, target_agents.len() as u32)
            .await?;

        info!(
            rollout_id = %rollout.id,
            bundle_id = %input.bundle_id,
            strategy = %strategy.strategy_type,
            target_agents = target_agents.len(),
            "Rollout created"
        );

        // Snapshot the pre-rollout decision-quality baseline (round-3 Plan 03),
        // so the decision-quality arm measures this rollout's SHIFT rather than
        // an absolute deny rate. Best-effort — never fails the rollout.
        self.capture_rollout_baseline(org_id, rollout.id).await;

        // Create waves based on strategy
        let waves = self
            .create_waves_for_strategy(&deploy_repo, &rollout, &strategy, &target_agents)
            .await?;

        // Broadcast rollout started event
        state.broadcast_event(ServerEvent::RolloutStarted {
            rollout_id: rollout.id,
            bundle_id: input.bundle_id,
            org_id,
            namespace_id: input.namespace_id,
        });

        // For immediate deployments, start execution right away
        if strategy.strategy_type == StrategyType::Immediate {
            self.execute_rollout_wave(&deploy_repo, &rollout, &waves[0], state)
                .await?;
        }

        Ok(RolloutResult {
            rollout,
            waves,
            target_agents,
        })
    }

    /// Get rollout status
    pub async fn get_rollout(&self, id: Uuid) -> Result<Rollout, DeploymentError> {
        let repo = DeploymentRepository::new(&self.db);
        repo.get_rollout_by_id(id)
            .await?
            .ok_or_else(|| DeploymentError::RolloutNotFound(id.to_string()))
    }

    /// Get rollout with waves
    pub async fn get_rollout_with_waves(
        &self,
        id: Uuid,
    ) -> Result<(Rollout, Vec<RolloutWave>), DeploymentError> {
        let repo = DeploymentRepository::new(&self.db);
        let rollout = repo
            .get_rollout_by_id(id)
            .await?
            .ok_or_else(|| DeploymentError::RolloutNotFound(id.to_string()))?;
        let waves = repo.get_waves_for_rollout(id).await?;
        Ok((rollout, waves))
    }

    /// List rollouts
    pub async fn list_rollouts(
        &self,
        org_id: Uuid,
        namespace_id: Option<Uuid>,
        limit: i32,
    ) -> Result<Vec<Rollout>, DeploymentError> {
        let repo = DeploymentRepository::new(&self.db);
        Ok(repo.list_rollouts(org_id, namespace_id, limit).await?)
    }

    /// List every ACTIVE rollout across all orgs with the owning org id —
    /// the rollout supervisor's per-tick work list (single indexed query).
    pub async fn list_active_rollouts_global(
        &self,
    ) -> Result<Vec<(Rollout, Uuid)>, DeploymentError> {
        let repo = DeploymentRepository::new(&self.db);
        Ok(repo.list_active_rollouts_global().await?)
    }

    /// Evaluate a rollout against its resolved auto-rollback configuration
    /// (namespace-specific, falling back to org-level, defaulting to
    /// disabled). The one source of truth for the trigger decision: the
    /// `check-rollback` and `rollback-status` endpoints and the autonomous
    /// rollout supervisor all call this.
    pub async fn evaluate_rollback_trigger(
        &self,
        org_id: Uuid,
        rollout: &Rollout,
    ) -> Result<RollbackTriggerEvaluation, DeploymentError> {
        // Resolve config: namespace-specific first, then org-level, then the
        // built-in default (disabled).
        let rollback_repo = RollbackConfigRepository::new(&self.db);
        let mut config = rollback_repo.get(org_id, rollout.namespace_id).await?;
        if config.is_none() {
            config = rollback_repo.get(org_id, None).await?;
        }
        let config = config.unwrap_or_else(|| RollbackConfig::new(org_id, rollout.namespace_id));

        if !config.is_enabled {
            return Ok(RollbackTriggerEvaluation {
                enabled: false,
                mode: config.mode,
                should_rollback: false,
                current_error_rate: 0.0,
                threshold: config.error_rate_threshold,
                completed_count: 0,
                min_requests: config.min_requests,
                reason: "Auto-rollback is disabled".to_string(),
            });
        }

        // --- Arm 1: deploy-apply failure rate across the rollout's agents. ---
        let deployment_repo = AgentDeploymentRepository::new(&self.db);
        let summary = deployment_repo.get_summary(rollout.id).await?;
        let completed_count = summary.deployed + summary.failed;
        let error_rate = summary.failure_rate();
        let deploy_apply_trip =
            completed_count >= config.min_requests && error_rate > config.error_rate_threshold;

        // --- Arm 2: decision quality (round-3 Plan 03). A policy can install on
        // every agent (deploy_apply_trip == false) yet error or deny wrongly at
        // runtime. Off unless a threshold is set, so today's behaviour is
        // unchanged for existing configs. ---
        let (dq_trip, dq_reason) = self
            .evaluate_decision_quality(org_id, rollout, &config)
            .await?;

        let should_rollback = deploy_apply_trip || dq_trip;

        let reason = if dq_trip {
            dq_reason
        } else if deploy_apply_trip {
            format!(
                "Error rate {:.2}% exceeds threshold {:.2}%",
                error_rate, config.error_rate_threshold
            )
        } else if completed_count < config.min_requests {
            format!(
                "Minimum requests not met ({} < {})",
                completed_count, config.min_requests
            )
        } else {
            format!(
                "Error rate {:.2}% within threshold {:.2}%",
                error_rate, config.error_rate_threshold
            )
        };

        Ok(RollbackTriggerEvaluation {
            enabled: true,
            mode: config.mode,
            should_rollback,
            current_error_rate: error_rate,
            threshold: config.error_rate_threshold,
            completed_count,
            min_requests: config.min_requests,
            reason,
        })
    }

    /// The decision-quality arm of the rollback trigger (round-3 Plan 03).
    /// Returns `(tripped, reason)`. Reads the org's live decision metrics from
    /// the heartbeat-fed `agent_metrics_latest` and trips when, gated by
    /// `min_decisions`:
    /// - the eval-error rate exceeds its absolute threshold (an eval-error is
    ///   never an intended outcome), OR
    /// - the fleet p99 eval latency exceeds its absolute SLO, OR
    /// - the deny rate has risen more than `denial_delta_threshold` ABOVE the
    ///   rollout's pre-rollout baseline (delta, so a legitimate policy change
    ///   that deliberately moves denies is not punished).
    async fn evaluate_decision_quality(
        &self,
        org_id: Uuid,
        rollout: &Rollout,
        config: &RollbackConfig,
    ) -> Result<(bool, String), DeploymentError> {
        let configured = config.eval_error_rate_threshold.is_some()
            || config.denial_delta_threshold.is_some()
            || config.latency_p99_slo_us.is_some();
        if !configured {
            return Ok((false, String::new()));
        }

        let dq = AgentRepository::new(&self.db)
            .aggregate_org_decision_metrics(org_id)
            .await?;

        // Thin traffic: never trip on too few decisions.
        if dq.total_decisions() < config.min_decisions as u64 {
            return Ok((false, String::new()));
        }

        if let Some(threshold) = config.eval_error_rate_threshold {
            if dq.eval_error_rate() > threshold {
                return Ok((
                    true,
                    format!(
                        "decision-quality: eval-error rate {:.2}% exceeds {:.2}%",
                        dq.eval_error_rate(),
                        threshold
                    ),
                ));
            }
        }

        if let Some(slo) = config.latency_p99_slo_us {
            if dq.p99_latency_us > slo {
                return Ok((
                    true,
                    format!(
                        "decision-quality: p99 latency {:.0}µs exceeds SLO {:.0}µs",
                        dq.p99_latency_us, slo
                    ),
                ));
            }
        }

        if let Some(delta) = config.denial_delta_threshold {
            // No baseline (no prior traffic) ⇒ use the current rate, so the
            // first observation can never trip on a delta of zero.
            let baseline = self
                .rollout_baseline_denial_rate(rollout.id)
                .await?
                .unwrap_or_else(|| dq.denial_rate());
            let shift = dq.denial_rate() - baseline;
            if shift > delta {
                return Ok((
                    true,
                    format!(
                        "decision-quality: deny rate {:.2}% is {:.2}pp above baseline {:.2}% (> {:.2}pp)",
                        dq.denial_rate(),
                        shift,
                        baseline,
                        delta
                    ),
                ));
            }
        }

        Ok((false, String::new()))
    }

    /// Read the pre-rollout denial-rate baseline captured at `start_rollout`.
    async fn rollout_baseline_denial_rate(
        &self,
        rollout_id: Uuid,
    ) -> Result<Option<f64>, DeploymentError> {
        let pool = self.db.any_pool().ok_or_else(|| {
            DeploymentError::Database(crate::db::DatabaseError::Config(
                "No database pool".to_string(),
            ))
        })?;
        let row: Option<(Option<f64>,)> =
            sqlx::query_as("SELECT baseline_denial_rate FROM rollouts WHERE id = $1")
                .bind(rollout_id.to_string())
                .fetch_optional(pool)
                .await
                .map_err(|e| DeploymentError::Database(e.into()))?;
        Ok(row.and_then(|r| r.0))
    }

    /// Capture the org's current deny rate as this rollout's decision-quality
    /// baseline (round-3 Plan 03). Best-effort: a metrics hiccup must not fail
    /// the rollout — the arm falls back to the current rate when absent.
    async fn capture_rollout_baseline(&self, org_id: Uuid, rollout_id: Uuid) {
        let denial_rate = match AgentRepository::new(&self.db)
            .aggregate_org_decision_metrics(org_id)
            .await
        {
            Ok(dq) if dq.total_decisions() > 0 => dq.denial_rate(),
            _ => return,
        };
        if let Some(pool) = self.db.any_pool() {
            let _ = sqlx::query("UPDATE rollouts SET baseline_denial_rate = $1 WHERE id = $2")
                .bind(denial_rate)
                .bind(rollout_id.to_string())
                .execute(pool)
                .await;
        }
    }

    /// Approve and proceed with next wave
    pub async fn approve_wave(
        &self,
        rollout_id: Uuid,
        state: &AppState,
    ) -> Result<Rollout, DeploymentError> {
        let repo = DeploymentRepository::new(&self.db);
        let rollout = self.get_rollout(rollout_id).await?;

        if !rollout.can_proceed() {
            return Err(DeploymentError::InvalidState(format!(
                "Rollout is not awaiting approval, status: {}",
                rollout.status
            )));
        }

        // Advance to next wave
        let rollout = repo.advance_wave(rollout_id).await?;
        let waves = repo.get_waves_for_rollout(rollout_id).await?;

        // Get the next wave
        let next_wave = waves
            .iter()
            .find(|w| w.wave_number == rollout.current_wave)
            .ok_or_else(|| {
                DeploymentError::InvalidState("No wave found for current wave number".to_string())
            })?;

        // Execute the wave
        self.execute_rollout_wave(&repo, &rollout, next_wave, state)
            .await?;

        self.get_rollout(rollout_id).await
    }

    /// Cancel a rollout
    pub async fn cancel_rollout(
        &self,
        rollout_id: Uuid,
        reason: &str,
        state: &AppState,
    ) -> Result<Rollout, DeploymentError> {
        let repo = DeploymentRepository::new(&self.db);
        let rollout = self.get_rollout(rollout_id).await?;

        if !rollout.can_cancel() {
            return Err(DeploymentError::InvalidState(format!(
                "Cannot cancel rollout in status: {}",
                rollout.status
            )));
        }

        let rollout = repo
            .update_rollout_status(rollout_id, RolloutStatus::Cancelled, Some(reason))
            .await?;

        info!(
            rollout_id = %rollout_id,
            reason = %reason,
            "Rollout cancelled"
        );

        // Broadcast completion event
        let bundle_repo = BundleRepository::new(&self.db);
        if let Ok(Some(bundle)) = bundle_repo.get_by_id(rollout.bundle_id).await {
            state.broadcast_event(ServerEvent::RolloutCompleted {
                rollout_id,
                bundle_id: rollout.bundle_id,
                org_id: bundle.org_id,
                namespace_id: rollout.namespace_id,
                success: false,
            });
        }

        Ok(rollout)
    }

    /// Rollback to previous bundle.
    ///
    /// `triggered_by` is a provenance marker stamped onto the rollback
    /// rollout's row (None for operator-initiated rollbacks); the rollout
    /// supervisor passes `AUTO_ROLLBACK_TRIGGER` so it can recognize — and
    /// never re-roll-back — its own remediation.
    pub async fn rollback(
        &self,
        org_id: Uuid,
        namespace_id: Option<Uuid>,
        target_bundle_id: Option<Uuid>,
        reason: &str,
        triggered_by: Option<&str>,
        state: &AppState,
    ) -> Result<RolloutResult, DeploymentError> {
        let bundle_repo = BundleRepository::new(&self.db);

        // Determine target bundle
        let target_bundle = if let Some(bundle_id) = target_bundle_id {
            bundle_repo
                .get_by_id(bundle_id)
                .await?
                .ok_or_else(|| DeploymentError::BundleNotFound(bundle_id.to_string()))?
        } else {
            // Get previous promoted bundle (this is a simplified version)
            let bundles = bundle_repo
                .list_by_org(org_id, Some(BundleStatus::Deprecated))
                .await?;
            bundles.into_iter().next().ok_or_else(|| {
                DeploymentError::BundleNotFound("No previous bundle found".to_string())
            })?
        };

        info!(
            target_bundle_id = %target_bundle.id,
            reason = %reason,
            "Initiating rollback"
        );

        // Start immediate rollout to the target bundle. The previous bundle
        // is typically Deprecated (promotion demoted it), which is exactly
        // what a rollback restores — so accept it here.
        let input = StartRollout {
            bundle_id: target_bundle.id,
            strategy_id: None, // Use immediate
            namespace_id,
            triggered_by: triggered_by.map(String::from),
        };

        self.start_rollout_with_options(org_id, &input, state, true)
            .await
    }

    // ==================== Version Pin Operations ====================

    /// Pin an agent to a specific bundle version
    pub async fn create_pin(
        &self,
        agent_id: Uuid,
        input: &CreateVersionPin,
        pinned_by: Option<&str>,
    ) -> Result<VersionPin, DeploymentError> {
        // Verify agent exists
        let agent_repo = AgentRepository::new(&self.db);
        agent_repo
            .get_by_id(agent_id)
            .await?
            .ok_or_else(|| DeploymentError::AgentNotFound(agent_id.to_string()))?;

        // Verify bundle exists
        let bundle_repo = BundleRepository::new(&self.db);
        bundle_repo
            .get_by_id(input.bundle_id)
            .await?
            .ok_or_else(|| DeploymentError::BundleNotFound(input.bundle_id.to_string()))?;

        let repo = DeploymentRepository::new(&self.db);
        let pin = repo.create_pin(agent_id, input, pinned_by).await?;

        info!(
            agent_id = %agent_id,
            bundle_id = %input.bundle_id,
            pinned_by = ?pinned_by,
            "Agent pinned to bundle version"
        );

        Ok(pin)
    }

    /// Get active pin for an agent
    pub async fn get_pin(&self, agent_id: Uuid) -> Result<Option<VersionPin>, DeploymentError> {
        let repo = DeploymentRepository::new(&self.db);
        Ok(repo.get_active_pin(agent_id).await?)
    }

    /// List all pins for an organization
    pub async fn list_pins(&self, org_id: Uuid) -> Result<Vec<VersionPin>, DeploymentError> {
        let repo = DeploymentRepository::new(&self.db);
        Ok(repo.list_pins(org_id).await?)
    }

    /// One keyset page of pins for an org (round-3 Plan 06 §4.2, R3-02).
    pub async fn list_pins_page(
        &self,
        org_id: Uuid,
        fetch: i64,
        after: Option<&(String, String)>,
    ) -> Result<Vec<VersionPin>, DeploymentError> {
        let repo = DeploymentRepository::new(&self.db);
        Ok(repo.list_pins_page(org_id, fetch, after).await?)
    }

    /// Remove a version pin
    pub async fn delete_pin(&self, agent_id: Uuid) -> Result<(), DeploymentError> {
        let repo = DeploymentRepository::new(&self.db);
        repo.delete_pin(agent_id).await?;
        info!(agent_id = %agent_id, "Agent version pin removed");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::FilesystemStorage;
    use std::collections::HashMap;
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
        .bind("Test Org")
        .bind("test-org")
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();
        org_id
    }

    async fn create_test_bundle(db: &Database, org_id: Uuid) -> Uuid {
        let pool = db.any_pool().unwrap();
        let bundle_id = Uuid::new_v4();
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO bundles (id, org_id, name, version, status, policy_count, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(bundle_id.to_string())
        .bind(org_id.to_string())
        .bind("test-bundle")
        .bind("1.0.0")
        .bind("compiled")
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
            let labels = if i == 0 {
                serde_json::json!({"env": "canary"})
            } else {
                serde_json::json!({"env": "production"})
            };

            sqlx::query(
                "INSERT INTO agents (id, org_id, name, status, labels, registered_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7)",
            )
            .bind(agent_id.to_string())
            .bind(org_id.to_string())
            .bind(format!("agent-{}", i))
            .bind("active")
            .bind(labels.to_string())
            .bind(&now)
            .bind(&now)
            .execute(pool)
            .await
            .unwrap();

            agent_ids.push(agent_id);
        }

        agent_ids
    }

    #[tokio::test]
    async fn test_create_strategy() {
        let (_temp_dir, db, _state) = setup().await;
        let org_id = create_test_org(&db).await;

        let service = DeploymentService::new(db);

        let input = CreateDeploymentStrategy {
            name: "canary-prod".to_string(),
            namespace_id: None,
            strategy_type: StrategyType::Canary,
            config: StrategyConfig::Canary {
                canary_labels: HashMap::from([("env".to_string(), "canary".to_string())]),
                wait_seconds: 300,
                require_approval: true,
            },
            is_default: true,
        };

        let strategy = service.create_strategy(org_id, &input).await.unwrap();
        assert_eq!(strategy.name, "canary-prod");
        assert_eq!(strategy.strategy_type, StrategyType::Canary);
    }

    #[tokio::test]
    async fn test_start_rollout_immediate() {
        let (_temp_dir, db, state) = setup().await;
        let org_id = create_test_org(&db).await;
        let bundle_id = create_test_bundle(&db, org_id).await;
        let _agents = create_test_agents(&db, org_id, 3).await;

        let service = DeploymentService::new(db);

        let input = StartRollout {
            bundle_id,
            strategy_id: None,
            namespace_id: None,
            triggered_by: None,
        };

        let result = service.start_rollout(org_id, &input, &state).await.unwrap();
        assert_eq!(result.waves.len(), 1);
        assert_eq!(result.target_agents.len(), 3);
    }

    #[tokio::test]
    async fn test_rollout_waits_for_and_completes_on_agent_confirmation() {
        let (_temp_dir, db, state) = setup().await;
        let org_id = create_test_org(&db).await;
        let bundle_id = create_test_bundle(&db, org_id).await;
        let agents = create_test_agents(&db, org_id, 3).await;

        let service = DeploymentService::new(db.clone());
        // Default behavior: wait for agent confirmations.
        assert!(service.require_agent_confirmation);

        let input = StartRollout {
            bundle_id,
            strategy_id: None,
            namespace_id: None,
            triggered_by: None,
        };
        let result = service.start_rollout(org_id, &input, &state).await.unwrap();
        let rollout_id = result.rollout.id;

        // The wave is deploying (NOT optimistically completed) and a pending
        // deployment row exists per target agent.
        let repo = DeploymentRepository::new(&db);
        let waves = repo.get_waves_for_rollout(rollout_id).await.unwrap();
        assert_eq!(waves.len(), 1);
        assert_eq!(waves[0].status, WaveStatus::Deploying);

        let dep_repo = AgentDeploymentRepository::new(&db);
        let summary = dep_repo.get_summary(rollout_id).await.unwrap();
        assert_eq!(summary.total_agents, 3);
        assert_eq!(summary.pending, 3);

        // Two of three confirm — rollout stays in progress.
        for a in &agents[..2] {
            service
                .record_agent_report(*a, bundle_id, AgentDeploymentStatus::Deployed, None, &state)
                .await
                .unwrap();
        }
        let rollout = repo.get_rollout_by_id(rollout_id).await.unwrap().unwrap();
        assert_ne!(rollout.status, RolloutStatus::Completed);
        assert_eq!(
            repo.get_waves_for_rollout(rollout_id).await.unwrap()[0].status,
            WaveStatus::Deploying
        );

        // Final agent confirms — the wave and the rollout complete.
        service
            .record_agent_report(
                agents[2],
                bundle_id,
                AgentDeploymentStatus::Deployed,
                None,
                &state,
            )
            .await
            .unwrap();

        let rollout = repo.get_rollout_by_id(rollout_id).await.unwrap().unwrap();
        assert_eq!(rollout.status, RolloutStatus::Completed);
        assert_eq!(
            repo.get_waves_for_rollout(rollout_id).await.unwrap()[0].status,
            WaveStatus::Completed
        );

        let summary = dep_repo.get_summary(rollout_id).await.unwrap();
        assert_eq!(summary.deployed, 3);
    }

    #[tokio::test]
    async fn test_report_records_untracked_deployment() {
        // A report for a bundle the plane wasn't tracking (e.g. direct promote)
        // still records the agent's actual applied state.
        let (_temp_dir, db, state) = setup().await;
        let org_id = create_test_org(&db).await;
        let bundle_id = create_test_bundle(&db, org_id).await;
        let agents = create_test_agents(&db, org_id, 1).await;

        let service = DeploymentService::new(db.clone());
        service
            .record_agent_report(
                agents[0],
                bundle_id,
                AgentDeploymentStatus::Deployed,
                None,
                &state,
            )
            .await
            .unwrap();

        let dep_repo = AgentDeploymentRepository::new(&db);
        let dep = dep_repo
            .get_latest_for_agent_bundle(agents[0], bundle_id)
            .await
            .unwrap()
            .expect("deployment recorded");
        assert_eq!(dep.status, AgentDeploymentStatus::Deployed);
        assert!(dep.acknowledged_at.is_some());
    }

    #[tokio::test]
    async fn test_evaluate_rollback_trigger_paths() {
        use crate::db::repositories::RollbackConfigRepository;
        use crate::domain::agent_deployment::{RollbackConfig, RollbackMode};

        let (_temp_dir, db, state) = setup().await;
        let org_id = create_test_org(&db).await;
        let bundle_id = create_test_bundle(&db, org_id).await;
        let agents = create_test_agents(&db, org_id, 4).await;
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
                &state,
            )
            .await
            .unwrap();
        let rollout = result.rollout;

        // 1. No config at all → built-in default is DISABLED.
        let eval = service
            .evaluate_rollback_trigger(org_id, &rollout)
            .await
            .unwrap();
        assert!(!eval.enabled);
        assert!(!eval.should_rollback);
        assert_eq!(eval.reason, "Auto-rollback is disabled");
        assert_eq!(eval.mode, RollbackMode::Monitor);

        // Enable at org level: threshold 50%, min 3 completed, enforce.
        let mut config = RollbackConfig::new(org_id, None);
        config.is_enabled = true;
        config.error_rate_threshold = 50.0;
        config.min_requests = 3;
        config.mode = RollbackMode::Enforce;
        RollbackConfigRepository::new(&db)
            .upsert(&config)
            .await
            .unwrap();

        // 2. Enabled but below min_requests (0 completed of 4).
        let eval = service
            .evaluate_rollback_trigger(org_id, &rollout)
            .await
            .unwrap();
        assert!(eval.enabled);
        assert!(!eval.should_rollback);
        assert_eq!(eval.completed_count, 0);
        assert!(eval.reason.contains("Minimum requests not met"));

        // 3. Enough data, below threshold: 2 deployed + 1 failed of 4 rows
        //    → 25% failure rate < 50%.
        let dep_repo = AgentDeploymentRepository::new(&db);
        let dep_for = |agent: Uuid, deps: &[AgentDeployment]| {
            deps.iter().find(|d| d.agent_id == agent).unwrap().id
        };
        let deps = dep_repo.get_by_rollout(rollout.id).await.unwrap();
        for a in &agents[..2] {
            dep_repo
                .update_status(dep_for(*a, &deps), AgentDeploymentStatus::Deployed, None)
                .await
                .unwrap();
        }
        dep_repo
            .update_status(
                dep_for(agents[2], &deps),
                AgentDeploymentStatus::Failed,
                Some("boom"),
            )
            .await
            .unwrap();

        let eval = service
            .evaluate_rollback_trigger(org_id, &rollout)
            .await
            .unwrap();
        assert!(eval.enabled);
        assert_eq!(eval.completed_count, 3);
        assert_eq!(eval.current_error_rate, 25.0);
        assert!(!eval.should_rollback);
        assert!(eval.reason.contains("within threshold"));

        // 4. Above threshold: 3 failed of 4 rows → 75% > 50% → fires, and
        //    the config's mode is carried through for the supervisor.
        for a in [agents[1], agents[3]] {
            dep_repo
                .update_status(
                    dep_for(a, &deps),
                    AgentDeploymentStatus::Failed,
                    Some("boom"),
                )
                .await
                .unwrap();
        }
        let eval = service
            .evaluate_rollback_trigger(org_id, &rollout)
            .await
            .unwrap();
        assert!(eval.should_rollback);
        assert_eq!(eval.current_error_rate, 75.0);
        assert_eq!(eval.threshold, 50.0);
        assert_eq!(eval.mode, RollbackMode::Enforce);
        assert!(eval.reason.contains("exceeds threshold"));
    }

    /// Round-3 Plan 03 DoD headline: a policy that installs cleanly on every
    /// agent (deploy-apply failure rate 0%) but then ERRORS on a large share of
    /// requests must trip the decision-quality arm and auto-revert — the exact
    /// failure the deploy-apply-only trigger missed.
    #[tokio::test]
    async fn test_decision_quality_rollback_on_eval_errors() {
        use crate::domain::agent::AgentMetrics;
        use crate::domain::agent_deployment::RollbackMode;
        let (_temp_dir, db, state) = setup().await;
        let org_id = create_test_org(&db).await;
        let bundle_id = create_test_bundle(&db, org_id).await;
        let agents = create_test_agents(&db, org_id, 3).await;
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
                &state,
            )
            .await
            .unwrap();
        let rollout = result.rollout;

        // Clean apply: every agent deployed, zero failures → the deploy-apply
        // arm can never trip (its whole blind spot).
        let dep_repo = AgentDeploymentRepository::new(&db);
        for d in dep_repo.get_by_rollout(rollout.id).await.unwrap() {
            dep_repo
                .update_status(d.id, AgentDeploymentStatus::Deployed, None)
                .await
                .unwrap();
        }

        // Deploy-apply threshold high (won't trip); decision-quality arm trips at
        // eval-error rate > 10%, enforce.
        let mut config = RollbackConfig::new(org_id, None);
        config.is_enabled = true;
        config.error_rate_threshold = 99.0;
        config.min_requests = 1;
        config.mode = RollbackMode::Enforce;
        config.eval_error_rate_threshold = Some(10.0);
        config.min_decisions = 10;
        RollbackConfigRepository::new(&db)
            .upsert(&config)
            .await
            .unwrap();

        let agent_repo = AgentRepository::new(&db);
        let set_metrics = |allow: u64, deny: u64, errs: u64| AgentMetrics {
            decisions_allow: allow,
            decisions_deny: deny,
            eval_errors: errs,
            ..Default::default()
        };

        // Healthy: no eval-errors → no trip, even though the policy is live.
        for a in &agents {
            agent_repo
                .update_metrics(*a, &set_metrics(90, 10, 0))
                .await
                .unwrap();
        }
        let eval = service
            .evaluate_rollback_trigger(org_id, &rollout)
            .await
            .unwrap();
        assert!(
            !eval.should_rollback,
            "healthy decisions must not trip: {}",
            eval.reason
        );

        // The deployed policy now errors on 40% of requests fleet-wide.
        for a in &agents {
            agent_repo
                .update_metrics(*a, &set_metrics(50, 10, 40))
                .await
                .unwrap();
        }
        let eval = service
            .evaluate_rollback_trigger(org_id, &rollout)
            .await
            .unwrap();
        assert!(
            eval.should_rollback,
            "an eval-error spike on a cleanly-applied policy must trip"
        );
        assert_eq!(
            eval.current_error_rate, 0.0,
            "deploy-apply failure rate is still zero — the clean-apply blind spot"
        );
        assert!(
            eval.reason.contains("eval-error rate"),
            "reason must name the decision-quality trip: {}",
            eval.reason
        );
        assert_eq!(eval.mode, RollbackMode::Enforce);

        // Thin traffic (9 decisions < min_decisions 10) must never trip.
        for a in &agents {
            agent_repo
                .update_metrics(*a, &set_metrics(1, 0, 2))
                .await
                .unwrap();
        }
        let eval = service
            .evaluate_rollback_trigger(org_id, &rollout)
            .await
            .unwrap();
        assert!(
            !eval.should_rollback,
            "below min_decisions must not trip (anti-storm guardrail)"
        );
    }

    #[tokio::test]
    async fn test_version_pin() {
        let (_temp_dir, db, _state) = setup().await;
        let org_id = create_test_org(&db).await;
        let bundle_id = create_test_bundle(&db, org_id).await;
        let agents = create_test_agents(&db, org_id, 1).await;
        let agent_id = agents[0];

        let service = DeploymentService::new(db);

        let input = CreateVersionPin {
            bundle_id,
            reason: Some("Testing".to_string()),
            expires_at: None,
        };

        let pin = service
            .create_pin(agent_id, &input, Some("admin"))
            .await
            .unwrap();
        assert_eq!(pin.bundle_id, bundle_id);

        let retrieved = service.get_pin(agent_id).await.unwrap();
        assert!(retrieved.is_some());

        service.delete_pin(agent_id).await.unwrap();
        let deleted = service.get_pin(agent_id).await.unwrap();
        assert!(deleted.is_none());
    }
}
