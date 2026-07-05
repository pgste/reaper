//! Deployment repository
//!
//! Data access layer for deployment strategies, rollouts, and version pins.

mod pins;
mod rollouts;
mod row_conversions;
mod strategies;
mod waves;

use uuid::Uuid;

use crate::db::{Database, DatabaseError};
use crate::domain::deployment::{
    CreateDeploymentStrategy, CreateVersionPin, DeploymentStrategy, Rollout, RolloutStatus,
    RolloutWave, StartRollout, VersionPin, WaveStatus,
};

use pins::PinOps;
use rollouts::RolloutOps;
use strategies::StrategyOps;
use waves::WaveOps;

/// Repository for deployment operations
pub struct DeploymentRepository<'a> {
    db: &'a Database,
}

impl<'a> DeploymentRepository<'a> {
    /// Create a new repository instance
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    // ==================== Deployment Strategies ====================

    fn strategies(&self) -> StrategyOps<'_> {
        StrategyOps { db: self.db }
    }

    /// Create a new deployment strategy
    pub async fn create_strategy(
        &self,
        org_id: Uuid,
        input: &CreateDeploymentStrategy,
    ) -> Result<DeploymentStrategy, DatabaseError> {
        self.strategies().create(org_id, input).await
    }

    /// Get a deployment strategy by ID
    pub async fn get_strategy_by_id(
        &self,
        id: Uuid,
    ) -> Result<Option<DeploymentStrategy>, DatabaseError> {
        self.strategies().get_by_id(id).await
    }

    /// List deployment strategies for an organization
    pub async fn list_strategies(
        &self,
        org_id: Uuid,
        namespace_id: Option<Uuid>,
    ) -> Result<Vec<DeploymentStrategy>, DatabaseError> {
        self.strategies().list(org_id, namespace_id).await
    }

    /// Get the default strategy for a namespace (or org-wide)
    pub async fn get_default_strategy(
        &self,
        org_id: Uuid,
        namespace_id: Option<Uuid>,
    ) -> Result<Option<DeploymentStrategy>, DatabaseError> {
        self.strategies().get_default(org_id, namespace_id).await
    }

    /// Delete a deployment strategy
    pub async fn delete_strategy(&self, id: Uuid) -> Result<(), DatabaseError> {
        self.strategies().delete(id).await
    }

    // ==================== Rollouts ====================

    fn rollouts(&self) -> RolloutOps<'_> {
        RolloutOps { db: self.db }
    }

    /// Create a new rollout
    pub async fn create_rollout(
        &self,
        input: &StartRollout,
        target_agent_count: u32,
    ) -> Result<Rollout, DatabaseError> {
        self.rollouts().create(input, target_agent_count).await
    }

    /// Get a rollout by ID
    pub async fn get_rollout_by_id(&self, id: Uuid) -> Result<Option<Rollout>, DatabaseError> {
        self.rollouts().get_by_id(id).await
    }

    /// Get active rollouts for a bundle
    pub async fn get_active_rollouts_for_bundle(
        &self,
        bundle_id: Uuid,
    ) -> Result<Vec<Rollout>, DatabaseError> {
        self.rollouts().get_active_for_bundle(bundle_id).await
    }

    /// List rollouts for a namespace
    pub async fn list_rollouts(
        &self,
        org_id: Uuid,
        namespace_id: Option<Uuid>,
        limit: i32,
    ) -> Result<Vec<Rollout>, DatabaseError> {
        self.rollouts().list(org_id, namespace_id, limit).await
    }

    /// Update rollout status
    pub async fn update_rollout_status(
        &self,
        id: Uuid,
        status: RolloutStatus,
        error: Option<&str>,
    ) -> Result<Rollout, DatabaseError> {
        self.rollouts().update_status(id, status, error).await
    }

    /// Increment deployed agent count
    pub async fn increment_deployed_count(
        &self,
        id: Uuid,
        count: u32,
    ) -> Result<Rollout, DatabaseError> {
        self.rollouts().increment_deployed_count(id, count).await
    }

    /// Advance to next wave
    pub async fn advance_wave(&self, id: Uuid) -> Result<Rollout, DatabaseError> {
        self.rollouts().advance_wave(id).await
    }

    // ==================== Rollout Waves ====================

    fn waves(&self) -> WaveOps<'_> {
        WaveOps { db: self.db }
    }

    /// Create a rollout wave
    pub async fn create_wave(
        &self,
        rollout_id: Uuid,
        wave_number: u32,
        target_agents: &[Uuid],
    ) -> Result<RolloutWave, DatabaseError> {
        self.waves()
            .create(rollout_id, wave_number, target_agents)
            .await
    }

    /// Get a wave by ID
    pub async fn get_wave_by_id(&self, id: Uuid) -> Result<Option<RolloutWave>, DatabaseError> {
        self.waves().get_by_id(id).await
    }

    /// Get waves for a rollout
    pub async fn get_waves_for_rollout(
        &self,
        rollout_id: Uuid,
    ) -> Result<Vec<RolloutWave>, DatabaseError> {
        self.waves().get_for_rollout(rollout_id).await
    }

    /// Update wave status
    pub async fn update_wave_status(
        &self,
        id: Uuid,
        status: WaveStatus,
    ) -> Result<RolloutWave, DatabaseError> {
        self.waves().update_status(id, status).await
    }

    /// Increment deployed count for a wave
    pub async fn increment_wave_deployed(
        &self,
        id: Uuid,
        count: u32,
    ) -> Result<RolloutWave, DatabaseError> {
        self.waves().increment_deployed(id, count).await
    }

    // ==================== Version Pins ====================

    fn pins(&self) -> PinOps<'_> {
        PinOps { db: self.db }
    }

    /// Create or update a version pin
    pub async fn create_pin(
        &self,
        agent_id: Uuid,
        input: &CreateVersionPin,
        pinned_by: Option<&str>,
    ) -> Result<VersionPin, DatabaseError> {
        self.pins().create(agent_id, input, pinned_by).await
    }

    /// Get a version pin for an agent
    pub async fn get_pin(&self, agent_id: Uuid) -> Result<Option<VersionPin>, DatabaseError> {
        self.pins().get(agent_id).await
    }

    /// Get active (non-expired) pin for an agent
    pub async fn get_active_pin(
        &self,
        agent_id: Uuid,
    ) -> Result<Option<VersionPin>, DatabaseError> {
        self.pins().get_active(agent_id).await
    }

    /// List all pins for agents in an org
    pub async fn list_pins(&self, org_id: Uuid) -> Result<Vec<VersionPin>, DatabaseError> {
        self.pins().list(org_id).await
    }

    /// Delete a version pin
    pub async fn delete_pin(&self, agent_id: Uuid) -> Result<(), DatabaseError> {
        self.pins().delete(agent_id).await
    }

    /// Delete expired pins
    pub async fn delete_expired_pins(&self) -> Result<u64, DatabaseError> {
        self.pins().delete_expired().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DatabaseConfig;
    use crate::domain::deployment::{StrategyConfig, StrategyType};
    use std::collections::HashMap;
    use tempfile::TempDir;

    async fn setup_db() -> (TempDir, std::sync::Arc<Database>) {
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
        (temp_dir, std::sync::Arc::new(db))
    }

    async fn create_test_org(db: &Database) -> Uuid {
        use chrono::Utc;
        let pool = db.any_pool().unwrap();
        let org_id = Uuid::new_v4();
        let now = Utc::now().to_rfc3339();
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
        use chrono::Utc;
        let pool = db.any_pool().unwrap();
        let bundle_id = Uuid::new_v4();
        let now = Utc::now().to_rfc3339();
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

    async fn create_test_agent(db: &Database, org_id: Uuid) -> Uuid {
        use chrono::Utc;
        let pool = db.any_pool().unwrap();
        let agent_id = Uuid::new_v4();
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO agents (id, org_id, name, status, registered_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(agent_id.to_string())
        .bind(org_id.to_string())
        .bind("test-agent")
        .bind("online")
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();
        agent_id
    }

    #[tokio::test]
    async fn test_create_and_get_strategy() {
        let (_temp_dir, db) = setup_db().await;
        let org_id = create_test_org(&db).await;
        let repo = DeploymentRepository::new(&db);

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

        let strategy = repo.create_strategy(org_id, &input).await.unwrap();
        assert_eq!(strategy.name, "canary-prod");
        assert_eq!(strategy.strategy_type, StrategyType::Canary);
        assert!(strategy.is_default);

        let retrieved = repo.get_strategy_by_id(strategy.id).await.unwrap().unwrap();
        assert_eq!(retrieved.name, "canary-prod");
    }

    #[tokio::test]
    async fn test_list_strategies() {
        let (_temp_dir, db) = setup_db().await;
        let org_id = create_test_org(&db).await;
        let repo = DeploymentRepository::new(&db);

        // Create two strategies
        repo.create_strategy(
            org_id,
            &CreateDeploymentStrategy {
                name: "immediate".to_string(),
                namespace_id: None,
                strategy_type: StrategyType::Immediate,
                config: StrategyConfig::Immediate {},
                is_default: true,
            },
        )
        .await
        .unwrap();

        repo.create_strategy(
            org_id,
            &CreateDeploymentStrategy {
                name: "percentage".to_string(),
                namespace_id: None,
                strategy_type: StrategyType::Percentage,
                config: StrategyConfig::Percentage {
                    waves: vec![10, 25, 50, 100],
                    wave_delay_seconds: 60,
                    require_approval: false,
                },
                is_default: false,
            },
        )
        .await
        .unwrap();

        let strategies = repo.list_strategies(org_id, None).await.unwrap();
        assert_eq!(strategies.len(), 2);
    }

    #[tokio::test]
    async fn test_create_and_update_rollout() {
        let (_temp_dir, db) = setup_db().await;
        let org_id = create_test_org(&db).await;
        let bundle_id = create_test_bundle(&db, org_id).await;
        let repo = DeploymentRepository::new(&db);

        let input = StartRollout {
            bundle_id,
            strategy_id: None,
            namespace_id: None,
        };

        let rollout = repo.create_rollout(&input, 10).await.unwrap();
        assert_eq!(rollout.status, RolloutStatus::Pending);
        assert_eq!(rollout.target_agent_count, 10);

        // Start the rollout
        let rollout = repo
            .update_rollout_status(rollout.id, RolloutStatus::InProgress, None)
            .await
            .unwrap();
        assert_eq!(rollout.status, RolloutStatus::InProgress);
        assert!(rollout.started_at.is_some());

        // Increment deployed count
        let rollout = repo.increment_deployed_count(rollout.id, 5).await.unwrap();
        assert_eq!(rollout.deployed_agent_count, 5);
    }

    #[tokio::test]
    async fn test_version_pins() {
        let (_temp_dir, db) = setup_db().await;
        let org_id = create_test_org(&db).await;
        let bundle_id = create_test_bundle(&db, org_id).await;
        let agent_id = create_test_agent(&db, org_id).await;
        let repo = DeploymentRepository::new(&db);

        let input = CreateVersionPin {
            bundle_id,
            reason: Some("Testing".to_string()),
            expires_at: None,
        };

        let pin = repo
            .create_pin(agent_id, &input, Some("admin"))
            .await
            .unwrap();
        assert_eq!(pin.bundle_id, bundle_id);
        assert_eq!(pin.pinned_by, Some("admin".to_string()));
        assert!(!pin.is_expired());

        // Get active pin
        let active_pin = repo.get_active_pin(agent_id).await.unwrap();
        assert!(active_pin.is_some());

        // Delete pin
        repo.delete_pin(agent_id).await.unwrap();
        let pin = repo.get_pin(agent_id).await.unwrap();
        assert!(pin.is_none());
    }

    #[tokio::test]
    async fn test_rollout_waves() {
        let (_temp_dir, db) = setup_db().await;
        let org_id = create_test_org(&db).await;
        let bundle_id = create_test_bundle(&db, org_id).await;
        let repo = DeploymentRepository::new(&db);

        let rollout = repo
            .create_rollout(
                &StartRollout {
                    bundle_id,
                    strategy_id: None,
                    namespace_id: None,
                },
                10,
            )
            .await
            .unwrap();

        let agent_ids = vec![Uuid::new_v4(), Uuid::new_v4()];
        let wave = repo.create_wave(rollout.id, 1, &agent_ids).await.unwrap();
        assert_eq!(wave.wave_number, 1);
        assert_eq!(wave.target_agents.len(), 2);
        assert_eq!(wave.status, WaveStatus::Pending);

        // Start deploying
        let wave = repo
            .update_wave_status(wave.id, WaveStatus::Deploying)
            .await
            .unwrap();
        assert_eq!(wave.status, WaveStatus::Deploying);
        assert!(wave.started_at.is_some());

        // Complete
        let wave = repo
            .update_wave_status(wave.id, WaveStatus::Completed)
            .await
            .unwrap();
        assert_eq!(wave.status, WaveStatus::Completed);
        assert!(wave.completed_at.is_some());
    }
}
