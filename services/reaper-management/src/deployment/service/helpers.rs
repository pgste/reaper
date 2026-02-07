//! Internal helper methods for DeploymentService

use std::collections::HashMap;
use tracing::{debug, info};
use uuid::Uuid;

use crate::db::repositories::{BundleRepository, DeploymentRepository};
use crate::domain::agent::Agent;
use crate::domain::deployment::{
    DeploymentStrategy, Rollout, RolloutStatus, RolloutWave, StrategyConfig, WaveStatus,
};
use crate::state::{AppState, ServerEvent};

use super::types::DeploymentError;
use super::DeploymentService;

impl DeploymentService {
    /// Select agents based on deployment strategy
    pub(super) fn select_agents_for_strategy(
        &self,
        strategy: &DeploymentStrategy,
        agents: &[Agent],
    ) -> Vec<Uuid> {
        match &strategy.config {
            StrategyConfig::Immediate {} => {
                // All agents
                agents.iter().map(|a| a.id).collect()
            }
            StrategyConfig::Canary { .. } => {
                // Filter by canary labels, then include all
                // For canary, we'll deploy to canary agents first, then all
                agents.iter().map(|a| a.id).collect()
            }
            StrategyConfig::Percentage { .. } => {
                // All agents (wave distribution happens later)
                agents.iter().map(|a| a.id).collect()
            }
            StrategyConfig::LabelSelector { labels } => {
                // Only agents matching labels
                agents
                    .iter()
                    .filter(|a| self.agent_matches_labels(a, labels))
                    .map(|a| a.id)
                    .collect()
            }
        }
    }

    /// Check if an agent matches the required labels
    pub(super) fn agent_matches_labels(
        &self,
        agent: &Agent,
        required_labels: &HashMap<String, String>,
    ) -> bool {
        if required_labels.is_empty() {
            return true;
        }

        let agent_labels = agent.labels.as_object();
        if agent_labels.is_none() {
            return false;
        }
        let agent_labels = agent_labels.unwrap();

        required_labels.iter().all(|(key, value)| {
            agent_labels
                .get(key)
                .and_then(|v| v.as_str())
                .map(|v| v == value)
                .unwrap_or(false)
        })
    }

    /// Create waves for a rollout based on strategy
    pub(super) async fn create_waves_for_strategy(
        &self,
        repo: &DeploymentRepository<'_>,
        rollout: &Rollout,
        strategy: &DeploymentStrategy,
        target_agents: &[Uuid],
    ) -> Result<Vec<RolloutWave>, DeploymentError> {
        use crate::db::repositories::AgentRepository;

        let mut waves = Vec::new();

        match &strategy.config {
            StrategyConfig::Immediate {} => {
                // Single wave with all agents
                let wave = repo.create_wave(rollout.id, 0, target_agents).await?;
                waves.push(wave);
            }
            StrategyConfig::Canary { canary_labels, .. } => {
                // Wave 0: canary agents
                // Wave 1: remaining agents
                let agent_repo = AgentRepository::new(&self.db);
                let all_agents = agent_repo
                    .list_by_org(rollout.bundle_id)
                    .await
                    .unwrap_or_default();

                let canary_agents: Vec<Uuid> = target_agents
                    .iter()
                    .filter(|id| {
                        all_agents
                            .iter()
                            .find(|a| a.id == **id)
                            .map(|a| self.agent_matches_labels(a, canary_labels))
                            .unwrap_or(false)
                    })
                    .cloned()
                    .collect();

                let remaining_agents: Vec<Uuid> = target_agents
                    .iter()
                    .filter(|id| !canary_agents.contains(id))
                    .cloned()
                    .collect();

                if !canary_agents.is_empty() {
                    let wave = repo.create_wave(rollout.id, 0, &canary_agents).await?;
                    waves.push(wave);
                }

                if !remaining_agents.is_empty() {
                    let wave = repo.create_wave(rollout.id, 1, &remaining_agents).await?;
                    waves.push(wave);
                }
            }
            StrategyConfig::Percentage {
                waves: percentages, ..
            } => {
                // Create waves based on percentages
                let total_agents = target_agents.len();
                let mut assigned = 0;

                for (i, pct) in percentages.iter().enumerate() {
                    let target_count = (total_agents * (*pct as usize) / 100).max(1);
                    let wave_agents: Vec<Uuid> = target_agents
                        .iter()
                        .skip(assigned)
                        .take(target_count - assigned.min(target_count))
                        .cloned()
                        .collect();

                    if !wave_agents.is_empty() {
                        let wave = repo.create_wave(rollout.id, i as u32, &wave_agents).await?;
                        waves.push(wave);
                        assigned += wave_agents.len();
                    }
                }

                // Ensure all agents are included in final wave
                if assigned < total_agents {
                    let remaining: Vec<Uuid> =
                        target_agents.iter().skip(assigned).cloned().collect();
                    let wave = repo
                        .create_wave(rollout.id, waves.len() as u32, &remaining)
                        .await?;
                    waves.push(wave);
                }
            }
            StrategyConfig::LabelSelector { .. } => {
                // Single wave with all matching agents
                let wave = repo.create_wave(rollout.id, 0, target_agents).await?;
                waves.push(wave);
            }
        }

        debug!(
            rollout_id = %rollout.id,
            wave_count = waves.len(),
            "Created rollout waves"
        );

        Ok(waves)
    }

    /// Execute a rollout wave (deploy to agents)
    pub(super) async fn execute_rollout_wave(
        &self,
        repo: &DeploymentRepository<'_>,
        rollout: &Rollout,
        wave: &RolloutWave,
        state: &AppState,
    ) -> Result<(), DeploymentError> {
        // Update rollout status to in progress
        repo.update_rollout_status(rollout.id, RolloutStatus::InProgress, None)
            .await?;

        // Update wave status
        repo.update_wave_status(wave.id, WaveStatus::Deploying)
            .await?;

        debug!(
            rollout_id = %rollout.id,
            wave_number = wave.wave_number,
            target_agents = wave.target_agents.len(),
            "Executing rollout wave"
        );

        // Get bundle details for the deployment event
        let bundle_repo = BundleRepository::new(&self.db);
        let bundle = bundle_repo.get_by_id(rollout.bundle_id).await?;

        if let Some(bundle) = bundle {
            // Broadcast bundle promoted event for each target agent
            let download_url = format!("/orgs/{}/bundles/{}/download", bundle.org_id, bundle.id);

            state.broadcast_event(ServerEvent::BundlePromoted {
                bundle_id: bundle.id,
                org_id: bundle.org_id,
                namespace_id: rollout.namespace_id,
                version: bundle
                    .checksum
                    .clone()
                    .unwrap_or_else(|| "1.0.0".to_string()),
                download_url,
            });
        }

        // Mark wave as completed (in a real system, this would wait for agent confirmations)
        repo.update_wave_status(wave.id, WaveStatus::Completed)
            .await?;

        // Update deployed count
        repo.increment_deployed_count(rollout.id, wave.target_agents.len() as u32)
            .await?;

        // Check if this was the last wave
        let waves = repo.get_waves_for_rollout(rollout.id).await?;
        let all_completed = waves.iter().all(|w| w.status == WaveStatus::Completed);

        if all_completed {
            // Mark rollout as completed
            repo.update_rollout_status(rollout.id, RolloutStatus::Completed, None)
                .await?;

            info!(
                rollout_id = %rollout.id,
                total_agents = rollout.target_agent_count,
                "Rollout completed successfully"
            );

            // Broadcast completion event
            if let Some(bundle) = bundle_repo.get_by_id(rollout.bundle_id).await? {
                state.broadcast_event(ServerEvent::RolloutCompleted {
                    rollout_id: rollout.id,
                    bundle_id: rollout.bundle_id,
                    org_id: bundle.org_id,
                    namespace_id: rollout.namespace_id,
                    success: true,
                });
            }
        } else {
            // Check strategy for approval requirement
            let deploy_repo = DeploymentRepository::new(&self.db);
            if let Some(strategy_id) = rollout.strategy_id {
                if let Some(strategy) = deploy_repo.get_strategy_by_id(strategy_id).await? {
                    let requires_approval = match &strategy.config {
                        StrategyConfig::Canary {
                            require_approval, ..
                        } => *require_approval,
                        StrategyConfig::Percentage {
                            require_approval, ..
                        } => *require_approval,
                        _ => false,
                    };

                    if requires_approval {
                        repo.update_rollout_status(
                            rollout.id,
                            RolloutStatus::AwaitingApproval,
                            None,
                        )
                        .await?;
                        info!(
                            rollout_id = %rollout.id,
                            wave = wave.wave_number,
                            "Rollout awaiting approval for next wave"
                        );
                    }
                }
            }

            // Broadcast wave completed event
            if let Some(bundle) = bundle_repo.get_by_id(rollout.bundle_id).await? {
                state.broadcast_event(ServerEvent::RolloutWaveCompleted {
                    rollout_id: rollout.id,
                    wave_number: wave.wave_number,
                    org_id: bundle.org_id,
                    namespace_id: rollout.namespace_id,
                });
            }
        }

        Ok(())
    }
}
