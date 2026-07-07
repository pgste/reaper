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
};
use crate::db::Database;
use crate::domain::agent::AgentStatus;
use crate::domain::agent_deployment::{AgentDeployment, AgentDeploymentStatus};
use crate::domain::bundle::BundleStatus;
use crate::domain::deployment::{
    CreateDeploymentStrategy, CreateVersionPin, DeploymentStrategy, Rollout, RolloutStatus,
    RolloutWave, StartRollout, StrategyConfig, StrategyType, VersionPin, WaveStatus,
};
use crate::state::{AppState, ServerEvent};

pub use types::{
    AgentInfo, DeploymentError, DryRunResult, RolloutResult, SkippedAgent, StrategyInfo,
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
        let bundle_repo = BundleRepository::new(&self.db);
        let deploy_repo = DeploymentRepository::new(&self.db);
        let agent_repo = AgentRepository::new(&self.db);

        // Verify bundle exists and is ready
        let bundle = bundle_repo
            .get_by_id(input.bundle_id)
            .await?
            .ok_or_else(|| DeploymentError::BundleNotFound(input.bundle_id.to_string()))?;

        if !matches!(
            bundle.status,
            BundleStatus::Compiled | BundleStatus::Staged | BundleStatus::Promoted
        ) {
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

    /// Rollback to previous bundle
    pub async fn rollback(
        &self,
        org_id: Uuid,
        namespace_id: Option<Uuid>,
        target_bundle_id: Option<Uuid>,
        reason: &str,
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

        // Start immediate rollout to the target bundle
        let input = StartRollout {
            bundle_id: target_bundle.id,
            strategy_id: None, // Use immediate
            namespace_id,
        };

        self.start_rollout(org_id, &input, state).await
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
