//! Team repository
//!
//! Provides CRUD operations for teams within organizations.

use crate::db::{Database, DatabaseError};
use crate::domain::team::{CreateTeam, Team, UpdateTeam};
use chrono::Utc;
use sqlx::Row;
use uuid::Uuid;

/// Repository for team operations
pub struct TeamRepository<'a> {
    db: &'a Database,
}

impl<'a> TeamRepository<'a> {
    /// Create a new team repository
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    /// Create a new team
    pub async fn create(&self, org_id: Uuid, input: CreateTeam) -> Result<Team, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let id = Uuid::new_v4();
        let now = Utc::now().to_rfc3339();

        sqlx::query(
            r#"
            INSERT INTO teams (id, org_id, name, slug, description, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            "#,
        )
        .bind(id.to_string())
        .bind(org_id.to_string())
        .bind(&input.name)
        .bind(&input.slug)
        .bind(&input.description)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await?;

        self.get_by_id(id).await?.ok_or_else(|| {
            DatabaseError::NotFound(format!("Team not found after creation: {}", id))
        })
    }

    /// Get a team by ID
    pub async fn get_by_id(&self, id: Uuid) -> Result<Option<Team>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let row = sqlx::query(
            r#"
            SELECT id, org_id, name, slug, description, created_at, updated_at
            FROM teams
            WHERE id = $1
            "#,
        )
        .bind(id.to_string())
        .fetch_optional(pool)
        .await?;

        match row {
            Some(row) => Ok(Some(self.row_to_team(row)?)),
            None => Ok(None),
        }
    }

    /// Get a team by org_id and slug
    pub async fn get_by_slug(
        &self,
        org_id: Uuid,
        slug: &str,
    ) -> Result<Option<Team>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let row = sqlx::query(
            r#"
            SELECT id, org_id, name, slug, description, created_at, updated_at
            FROM teams
            WHERE org_id = $1 AND slug = $2
            "#,
        )
        .bind(org_id.to_string())
        .bind(slug)
        .fetch_optional(pool)
        .await?;

        match row {
            Some(row) => Ok(Some(self.row_to_team(row)?)),
            None => Ok(None),
        }
    }

    /// List teams for an organization
    pub async fn list_by_org(
        &self,
        org_id: Uuid,
        limit: Option<i64>,
        offset: Option<i64>,
    ) -> Result<Vec<Team>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let limit = limit.unwrap_or(100);
        let offset = offset.unwrap_or(0);

        let rows = sqlx::query(
            r#"
            SELECT id, org_id, name, slug, description, created_at, updated_at
            FROM teams
            WHERE org_id = $1
            ORDER BY created_at DESC
            LIMIT $2 OFFSET $3
            "#,
        )
        .bind(org_id.to_string())
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?;

        let mut teams = Vec::with_capacity(rows.len());
        for row in rows {
            teams.push(self.row_to_team(row)?);
        }

        Ok(teams)
    }

    /// Count teams for an organization
    pub async fn count_by_org(&self, org_id: Uuid) -> Result<i64, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM teams WHERE org_id = $1")
            .bind(org_id.to_string())
            .fetch_one(pool)
            .await?;

        Ok(row.0)
    }

    /// Update a team
    pub async fn update(&self, id: Uuid, input: UpdateTeam) -> Result<Option<Team>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        // Get current team to merge updates
        let current = match self.get_by_id(id).await? {
            Some(team) => team,
            None => return Ok(None),
        };

        let now = Utc::now().to_rfc3339();
        let name = input.name.unwrap_or(current.name);
        let description = input.description.or(current.description);

        sqlx::query(
            r#"
            UPDATE teams
            SET name = $1, description = $2, updated_at = $3
            WHERE id = $4
            "#,
        )
        .bind(&name)
        .bind(&description)
        .bind(&now)
        .bind(id.to_string())
        .execute(pool)
        .await?;

        self.get_by_id(id).await
    }

    /// Delete a team
    pub async fn delete(&self, id: Uuid) -> Result<bool, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let result = sqlx::query("DELETE FROM teams WHERE id = $1")
            .bind(id.to_string())
            .execute(pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Convert a database row to a Team
    fn row_to_team(&self, row: sqlx::any::AnyRow) -> Result<Team, DatabaseError> {
        let id_str: String = row.get("id");
        let id = Uuid::parse_str(&id_str)
            .map_err(|e| DatabaseError::Config(format!("Invalid UUID: {}", e)))?;

        let org_id_str: String = row.get("org_id");
        let org_id = Uuid::parse_str(&org_id_str)
            .map_err(|e| DatabaseError::Config(format!("Invalid org UUID: {}", e)))?;

        let created_at_str: String = row.get("created_at");
        let created_at = chrono::DateTime::parse_from_rfc3339(&created_at_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());

        let updated_at_str: String = row.get("updated_at");
        let updated_at = chrono::DateTime::parse_from_rfc3339(&updated_at_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());

        Ok(Team {
            id,
            org_id,
            name: row.get("name"),
            slug: row.get("slug"),
            description: row.get("description"),
            created_at,
            updated_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DatabaseConfig;
    use crate::db::repositories::OrganizationRepository;
    use crate::domain::organization::CreateOrganization;
    use tempfile::TempDir;

    async fn setup_db() -> (TempDir, Database) {
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
        (temp_dir, db)
    }

    async fn create_test_org(db: &Database) -> Uuid {
        let repo = OrganizationRepository::new(db);
        let input = CreateOrganization {
            name: "Test Org".to_string(),
            slug: "test-org".to_string(),
            display_name: None,
            description: None,
            settings: serde_json::json!({}),
        };
        repo.create(input).await.unwrap().id
    }

    #[tokio::test]
    async fn test_create_team() {
        let (_temp_dir, db) = setup_db().await;
        let org_id = create_test_org(&db).await;
        let repo = TeamRepository::new(&db);

        let input = CreateTeam {
            name: "Engineering".to_string(),
            slug: "engineering".to_string(),
            description: Some("Engineering team".to_string()),
        };

        let team = repo.create(org_id, input).await.unwrap();
        assert_eq!(team.name, "Engineering");
        assert_eq!(team.slug, "engineering");
        assert_eq!(team.org_id, org_id);
    }

    #[tokio::test]
    async fn test_get_by_slug() {
        let (_temp_dir, db) = setup_db().await;
        let org_id = create_test_org(&db).await;
        let repo = TeamRepository::new(&db);

        let input = CreateTeam {
            name: "Platform".to_string(),
            slug: "platform".to_string(),
            description: None,
        };

        repo.create(org_id, input).await.unwrap();

        let team = repo.get_by_slug(org_id, "platform").await.unwrap().unwrap();
        assert_eq!(team.slug, "platform");
    }

    #[tokio::test]
    async fn test_list_teams() {
        let (_temp_dir, db) = setup_db().await;
        let org_id = create_test_org(&db).await;
        let repo = TeamRepository::new(&db);

        for i in 0..3 {
            let input = CreateTeam {
                name: format!("Team {}", i),
                slug: format!("team-{}", i),
                description: None,
            };
            repo.create(org_id, input).await.unwrap();
        }

        let teams = repo.list_by_org(org_id, None, None).await.unwrap();
        assert_eq!(teams.len(), 3);

        let count = repo.count_by_org(org_id).await.unwrap();
        assert_eq!(count, 3);
    }
}
