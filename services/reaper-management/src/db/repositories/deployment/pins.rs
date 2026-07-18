//! Version pin repository operations

use chrono::Utc;
use uuid::Uuid;

use crate::db::{Database, DatabaseError};
use crate::domain::deployment::{CreateVersionPin, VersionPin};

use super::row_conversions::row_to_pin;

/// Version pin repository operations
pub struct PinOps<'a> {
    pub(super) db: &'a Database,
}

impl<'a> PinOps<'a> {
    /// Create or update a version pin
    pub async fn create(
        &self,
        agent_id: Uuid,
        input: &CreateVersionPin,
        pinned_by: Option<&str>,
    ) -> Result<VersionPin, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let now = Utc::now();

        let sql = r#"
            INSERT INTO version_pins (agent_id, bundle_id, pinned_by, reason, expires_at, created_at)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT(agent_id) DO UPDATE SET
                bundle_id = excluded.bundle_id,
                pinned_by = excluded.pinned_by,
                reason = excluded.reason,
                expires_at = excluded.expires_at,
                created_at = excluded.created_at
        "#;

        sqlx::query(sql)
            .bind(agent_id.to_string())
            .bind(input.bundle_id.to_string())
            .bind(pinned_by)
            .bind(&input.reason)
            .bind(input.expires_at.map(|dt| dt.to_rfc3339()))
            .bind(now.to_rfc3339())
            .execute(pool)
            .await?;

        self.get(agent_id)
            .await?
            .ok_or_else(|| DatabaseError::NotFound("Pin not found after creation".to_string()))
    }

    /// Get a version pin for an agent
    pub async fn get(&self, agent_id: Uuid) -> Result<Option<VersionPin>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let sql = r#"
            SELECT agent_id, bundle_id, pinned_by, reason, expires_at, created_at
            FROM version_pins
            WHERE agent_id = $1
        "#;

        let row = sqlx::query(sql)
            .bind(agent_id.to_string())
            .fetch_optional(pool)
            .await?;

        row.map(|r| row_to_pin(&r)).transpose()
    }

    /// Get active (non-expired) pin for an agent
    pub async fn get_active(&self, agent_id: Uuid) -> Result<Option<VersionPin>, DatabaseError> {
        let pin = self.get(agent_id).await?;
        Ok(pin.filter(|p| !p.is_expired()))
    }

    /// List all pins for agents in an org
    pub async fn list(&self, org_id: Uuid) -> Result<Vec<VersionPin>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let sql = r#"
            SELECT vp.agent_id, vp.bundle_id, vp.pinned_by, vp.reason, vp.expires_at, vp.created_at
            FROM version_pins vp
            INNER JOIN agents a ON vp.agent_id = a.id
            WHERE a.org_id = $1
        "#;

        let rows = sqlx::query(sql)
            .bind(org_id.to_string())
            .fetch_all(pool)
            .await?;

        rows.iter().map(row_to_pin).collect()
    }

    /// One keyset page of pins for an org, newest first (round-3 Plan 06 §4.2,
    /// R3-02). Pins are fleet-cardinality (one per pinned agent), so the
    /// unbounded `list` returned the whole set in one array at scale. Mirrors
    /// the proven `agents` keyset over `(created_at, agent_id)`.
    pub async fn list_page_by_org(
        &self,
        org_id: Uuid,
        fetch: i64,
        after: Option<&(String, String)>,
    ) -> Result<Vec<VersionPin>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let rows = if let Some((created_at, agent_id)) = after {
            sqlx::query(
                r#"
                SELECT vp.agent_id, vp.bundle_id, vp.pinned_by, vp.reason, vp.expires_at, vp.created_at
                FROM version_pins vp
                INNER JOIN agents a ON vp.agent_id = a.id
                WHERE a.org_id = $1 AND (vp.created_at, vp.agent_id) < ($2, $3)
                ORDER BY vp.created_at DESC, vp.agent_id DESC
                LIMIT $4
                "#,
            )
            .bind(org_id.to_string())
            .bind(created_at)
            .bind(agent_id)
            .bind(fetch)
            .fetch_all(pool)
            .await?
        } else {
            sqlx::query(
                r#"
                SELECT vp.agent_id, vp.bundle_id, vp.pinned_by, vp.reason, vp.expires_at, vp.created_at
                FROM version_pins vp
                INNER JOIN agents a ON vp.agent_id = a.id
                WHERE a.org_id = $1
                ORDER BY vp.created_at DESC, vp.agent_id DESC
                LIMIT $2
                "#,
            )
            .bind(org_id.to_string())
            .bind(fetch)
            .fetch_all(pool)
            .await?
        };

        rows.iter().map(row_to_pin).collect()
    }

    /// Delete a version pin
    pub async fn delete(&self, agent_id: Uuid) -> Result<(), DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let sql = "DELETE FROM version_pins WHERE agent_id = $1";
        let result = sqlx::query(sql)
            .bind(agent_id.to_string())
            .execute(pool)
            .await?;

        if result.rows_affected() == 0 {
            return Err(DatabaseError::NotFound(format!(
                "Pin for agent {} not found",
                agent_id
            )));
        }

        Ok(())
    }

    /// Delete expired pins
    pub async fn delete_expired(&self) -> Result<u64, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let now = Utc::now();

        let sql = "DELETE FROM version_pins WHERE expires_at IS NOT NULL AND expires_at < $1";
        let result = sqlx::query(sql)
            .bind(now.to_rfc3339())
            .execute(pool)
            .await?;

        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::repositories::{
        AgentRepository, BundleRepository, DeploymentRepository, OrganizationRepository,
    };
    use crate::domain::agent::RegisterAgent;
    use crate::domain::bundle::CreateBundle;
    use crate::domain::organization::CreateOrganization;
    use std::collections::BTreeSet;
    use std::sync::Arc;

    /// The keyset page walk (round-3 Plan 06 §4.2, R3-02): with N pins and a
    /// page size < N, walking `list_page_by_org` by cursor visits every pin
    /// exactly once and terminates — the guard against the old unbounded
    /// `list`. Also pins the risk-prone bit: the cursor's `created_at` is
    /// `to_rfc3339()` and the column is TEXT, so the lexical keyset must be
    /// chronological.
    #[tokio::test]
    async fn pins_keyset_page_walk_visits_each_pin_once() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db_config = crate::db::ephemeral_test_config(tmp.path()).await;
        let db = Arc::new(Database::new(&db_config).await.unwrap());
        db.run_migrations().await.unwrap();

        let org = OrganizationRepository::new(&db)
            .create(CreateOrganization {
                name: "Pins Org".into(),
                slug: "pins-org".into(),
                display_name: None,
                description: None,
                settings: serde_json::json!({}),
            })
            .await
            .unwrap();

        // One bundle every pin references (FK), then N agents + one pin each
        // (the pin PK is agent_id, so N pins need N agents).
        let bundle = BundleRepository::new(&db)
            .create(
                org.id,
                &CreateBundle {
                    name: "b".into(),
                    description: None,
                    policy_ids: vec![],
                },
            )
            .await
            .unwrap();

        let deploy = DeploymentRepository::new(&db);
        let n = 25usize;
        let mut created = BTreeSet::new();
        for i in 0..n {
            let agent = AgentRepository::new(&db)
                .create(
                    org.id,
                    RegisterAgent {
                        name: format!("agent-{i}"),
                        hostname: None,
                        version: None,
                        labels: serde_json::json!({}),
                    },
                )
                .await
                .unwrap();
            deploy
                .pins()
                .create(
                    agent.id,
                    &CreateVersionPin {
                        bundle_id: bundle.id,
                        reason: Some(format!("pin-{i}")),
                        expires_at: None,
                    },
                    Some("tester"),
                )
                .await
                .unwrap();
            created.insert(agent.id);
        }

        // Walk with page size 7 (< 25): collect every pin exactly once.
        let page_size = 7i64;
        let mut seen: BTreeSet<Uuid> = BTreeSet::new();
        let mut after: Option<(String, String)> = None;
        let mut pages = 0;
        loop {
            let rows = deploy
                .list_pins_page(org.id, page_size + 1, after.as_ref())
                .await
                .unwrap();
            let has_more = rows.len() as i64 > page_size;
            let page: Vec<_> = rows.into_iter().take(page_size as usize).collect();
            for pin in &page {
                assert!(
                    seen.insert(pin.agent_id),
                    "keyset walk must never repeat a pin"
                );
            }
            match (has_more, page.last()) {
                (true, Some(last)) => {
                    after = Some((last.created_at.to_rfc3339(), last.agent_id.to_string()));
                }
                _ => break,
            }
            pages += 1;
            assert!(pages < 100, "cursor walk did not terminate");
        }

        assert_eq!(seen, created, "every pin is visited exactly once");
    }
}
