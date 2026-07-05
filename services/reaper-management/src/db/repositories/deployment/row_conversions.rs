//! Row conversion helpers for deployment repository

use chrono::Utc;
use sqlx::Row;
use uuid::Uuid;

use crate::db::DatabaseError;
use crate::domain::deployment::{
    DeploymentStrategy, Rollout, RolloutStatus, RolloutWave, StrategyConfig, StrategyType,
    VersionPin, WaveStatus,
};

/// Convert a SQLite row to a DeploymentStrategy
pub(super) fn row_to_strategy(
    row: &sqlx::any::AnyRow,
) -> Result<DeploymentStrategy, DatabaseError> {
    let id: String = row.get("id");
    let org_id: String = row.get("org_id");
    let namespace_id: Option<String> = row.get("namespace_id");
    let strategy_type: String = row.get("strategy_type");
    let config_json: String = row.get("config");
    let is_default: i32 = row.get("is_default");
    let created_at: String = row.get("created_at");
    let updated_at: String = row.get("updated_at");

    let config: StrategyConfig = serde_json::from_str(&config_json)
        .map_err(|e| DatabaseError::Config(format!("Failed to parse config: {}", e)))?;

    Ok(DeploymentStrategy {
        id: id
            .parse()
            .map_err(|e| DatabaseError::Config(format!("Invalid UUID: {}", e)))?,
        org_id: org_id
            .parse()
            .map_err(|e| DatabaseError::Config(format!("Invalid UUID: {}", e)))?,
        namespace_id: namespace_id
            .map(|s| {
                s.parse()
                    .map_err(|e| DatabaseError::Config(format!("Invalid UUID: {}", e)))
            })
            .transpose()?,
        name: row.get("name"),
        strategy_type: strategy_type.parse().unwrap_or(StrategyType::Immediate),
        config,
        is_default: is_default != 0,
        created_at: chrono::DateTime::parse_from_rfc3339(&created_at)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now()),
        updated_at: chrono::DateTime::parse_from_rfc3339(&updated_at)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now()),
    })
}

/// Convert a SQLite row to a Rollout
pub(super) fn row_to_rollout(row: &sqlx::any::AnyRow) -> Result<Rollout, DatabaseError> {
    let id: String = row.get("id");
    let bundle_id: String = row.get("bundle_id");
    let strategy_id: Option<String> = row.get("strategy_id");
    let namespace_id: Option<String> = row.get("namespace_id");
    let status: String = row.get("status");
    let started_at: Option<String> = row.get("started_at");
    let completed_at: Option<String> = row.get("completed_at");
    let created_at: String = row.get("created_at");
    let updated_at: String = row.get("updated_at");

    Ok(Rollout {
        id: id
            .parse()
            .map_err(|e| DatabaseError::Config(format!("Invalid UUID: {}", e)))?,
        bundle_id: bundle_id
            .parse()
            .map_err(|e| DatabaseError::Config(format!("Invalid UUID: {}", e)))?,
        strategy_id: strategy_id
            .map(|s| {
                s.parse()
                    .map_err(|e| DatabaseError::Config(format!("Invalid UUID: {}", e)))
            })
            .transpose()?,
        namespace_id: namespace_id
            .map(|s| {
                s.parse()
                    .map_err(|e| DatabaseError::Config(format!("Invalid UUID: {}", e)))
            })
            .transpose()?,
        status: status.parse().unwrap_or(RolloutStatus::Pending),
        current_wave: row.get::<i32, _>("current_wave") as u32,
        target_agent_count: row.get::<i32, _>("target_agent_count") as u32,
        deployed_agent_count: row.get::<i32, _>("deployed_agent_count") as u32,
        started_at: started_at.and_then(|s| {
            chrono::DateTime::parse_from_rfc3339(&s)
                .ok()
                .map(|dt| dt.with_timezone(&Utc))
        }),
        completed_at: completed_at.and_then(|s| {
            chrono::DateTime::parse_from_rfc3339(&s)
                .ok()
                .map(|dt| dt.with_timezone(&Utc))
        }),
        error: row.get("error"),
        created_at: chrono::DateTime::parse_from_rfc3339(&created_at)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now()),
        updated_at: chrono::DateTime::parse_from_rfc3339(&updated_at)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now()),
    })
}

/// Convert a SQLite row to a RolloutWave
pub(super) fn row_to_wave(row: &sqlx::any::AnyRow) -> Result<RolloutWave, DatabaseError> {
    let id: String = row.get("id");
    let rollout_id: String = row.get("rollout_id");
    let target_agents_json: String = row.get("target_agents");
    let status: String = row.get("status");
    let started_at: Option<String> = row.get("started_at");
    let completed_at: Option<String> = row.get("completed_at");
    let created_at: String = row.get("created_at");

    let target_agents: Vec<Uuid> = serde_json::from_str(&target_agents_json)
        .map_err(|e| DatabaseError::Config(format!("Failed to parse target_agents: {}", e)))?;

    Ok(RolloutWave {
        id: id
            .parse()
            .map_err(|e| DatabaseError::Config(format!("Invalid UUID: {}", e)))?,
        rollout_id: rollout_id
            .parse()
            .map_err(|e| DatabaseError::Config(format!("Invalid UUID: {}", e)))?,
        wave_number: row.get::<i32, _>("wave_number") as u32,
        target_agents,
        status: status.parse().unwrap_or(WaveStatus::Pending),
        deployed_count: row.get::<i32, _>("deployed_count") as u32,
        started_at: started_at.and_then(|s| {
            chrono::DateTime::parse_from_rfc3339(&s)
                .ok()
                .map(|dt| dt.with_timezone(&Utc))
        }),
        completed_at: completed_at.and_then(|s| {
            chrono::DateTime::parse_from_rfc3339(&s)
                .ok()
                .map(|dt| dt.with_timezone(&Utc))
        }),
        created_at: chrono::DateTime::parse_from_rfc3339(&created_at)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now()),
    })
}

/// Convert a SQLite row to a VersionPin
pub(super) fn row_to_pin(row: &sqlx::any::AnyRow) -> Result<VersionPin, DatabaseError> {
    let agent_id: String = row.get("agent_id");
    let bundle_id: String = row.get("bundle_id");
    let expires_at: Option<String> = row.get("expires_at");
    let created_at: String = row.get("created_at");

    Ok(VersionPin {
        agent_id: agent_id
            .parse()
            .map_err(|e| DatabaseError::Config(format!("Invalid UUID: {}", e)))?,
        bundle_id: bundle_id
            .parse()
            .map_err(|e| DatabaseError::Config(format!("Invalid UUID: {}", e)))?,
        pinned_by: row.get("pinned_by"),
        reason: row.get("reason"),
        expires_at: expires_at.and_then(|s| {
            chrono::DateTime::parse_from_rfc3339(&s)
                .ok()
                .map(|dt| dt.with_timezone(&Utc))
        }),
        created_at: chrono::DateTime::parse_from_rfc3339(&created_at)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now()),
    })
}
