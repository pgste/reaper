//! Webhook subscription repository
//!
//! CRUD operations for outbound webhook subscriptions.

use chrono::Utc;
use uuid::Uuid;

use crate::db::{Database, DatabaseError};
use crate::domain::webhook::{
    CreateWebhookSubscription, UpdateWebhookSubscription, WebhookEventType, WebhookSubscription,
};

/// Repository for webhook subscriptions
pub struct WebhookRepository<'a> {
    db: &'a Database,
}

impl<'a> WebhookRepository<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    /// Create a new webhook subscription
    pub async fn create(
        &self,
        org_id: Uuid,
        input: CreateWebhookSubscription,
    ) -> Result<WebhookSubscription, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Connection(sqlx::Error::PoolClosed))?;

        let id = Uuid::new_v4();
        let now = Utc::now();

        // Serialize events to JSON
        let events_json = serde_json::to_string(&input.events)
            .map_err(|e| DatabaseError::Migration(format!("Failed to serialize events: {}", e)))?;

        sqlx::query(
            r#"
            INSERT INTO webhook_subscriptions
            (id, org_id, name, url, secret, events, is_active, failure_count, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, 1, 0, $7, $8)
            "#,
        )
        .bind(id.to_string())
        .bind(org_id.to_string())
        .bind(&input.name)
        .bind(&input.url)
        .bind(&input.secret)
        .bind(&events_json)
        .bind(now.to_rfc3339())
        .bind(now.to_rfc3339())
        .execute(pool)
        .await?;

        Ok(WebhookSubscription {
            id,
            org_id,
            name: input.name,
            url: input.url,
            secret: input.secret,
            events: input.events,
            is_active: true,
            last_triggered_at: None,
            failure_count: 0,
            created_at: now,
            updated_at: now,
        })
    }

    /// Get a webhook subscription by ID
    pub async fn get_by_id(&self, id: Uuid) -> Result<Option<WebhookSubscription>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Connection(sqlx::Error::PoolClosed))?;

        let row: Option<WebhookRow> = sqlx::query_as(
            r#"
            SELECT id, org_id, name, url, secret, events, is_active,
                   last_triggered_at, failure_count, created_at, updated_at
            FROM webhook_subscriptions
            WHERE id = $1
            "#,
        )
        .bind(id.to_string())
        .fetch_optional(pool)
        .await?;

        row.map(|r| r.try_into()).transpose()
    }

    /// Get a webhook subscription by name within an org
    pub async fn get_by_name(
        &self,
        org_id: Uuid,
        name: &str,
    ) -> Result<Option<WebhookSubscription>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Connection(sqlx::Error::PoolClosed))?;

        let row: Option<WebhookRow> = sqlx::query_as(
            r#"
            SELECT id, org_id, name, url, secret, events, is_active,
                   last_triggered_at, failure_count, created_at, updated_at
            FROM webhook_subscriptions
            WHERE org_id = $1 AND name = $2
            "#,
        )
        .bind(org_id.to_string())
        .bind(name)
        .fetch_optional(pool)
        .await?;

        row.map(|r| r.try_into()).transpose()
    }

    /// List webhook subscriptions for an organization
    pub async fn list_by_org(
        &self,
        org_id: Uuid,
        active_only: bool,
        limit: i64,
    ) -> Result<Vec<WebhookSubscription>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Connection(sqlx::Error::PoolClosed))?;

        // Bounded cap so the list is never unbounded (round-3 Plan 06 §4.2).
        let query = if active_only {
            r#"
            SELECT id, org_id, name, url, secret, events, is_active,
                   last_triggered_at, failure_count, created_at, updated_at
            FROM webhook_subscriptions
            WHERE org_id = $1 AND is_active = 1
            ORDER BY name
            LIMIT $2
            "#
        } else {
            r#"
            SELECT id, org_id, name, url, secret, events, is_active,
                   last_triggered_at, failure_count, created_at, updated_at
            FROM webhook_subscriptions
            WHERE org_id = $1
            ORDER BY name
            LIMIT $2
            "#
        };

        let rows: Vec<WebhookRow> = sqlx::query_as(query)
            .bind(org_id.to_string())
            .bind(limit)
            .fetch_all(pool)
            .await?;

        rows.into_iter().map(|r| r.try_into()).collect()
    }

    /// List subscriptions for a specific event type
    pub async fn list_by_event(
        &self,
        org_id: Uuid,
        event: WebhookEventType,
    ) -> Result<Vec<WebhookSubscription>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Connection(sqlx::Error::PoolClosed))?;

        // SQLite JSON contains check
        let event_str = format!("\"{}\"", event);

        let rows: Vec<WebhookRow> = sqlx::query_as(
            r#"
            SELECT id, org_id, name, url, secret, events, is_active,
                   last_triggered_at, failure_count, created_at, updated_at
            FROM webhook_subscriptions
            WHERE org_id = $1 AND is_active = 1 AND events LIKE '%' || $2 || '%'
            ORDER BY name
            "#,
        )
        .bind(org_id.to_string())
        .bind(&event_str)
        .fetch_all(pool)
        .await?;

        // Filter more precisely - the LIKE is a quick pre-filter
        let subscriptions: Vec<WebhookSubscription> = rows
            .into_iter()
            .filter_map(|r| r.try_into().ok())
            .filter(|s: &WebhookSubscription| s.events.contains(&event))
            .collect();

        Ok(subscriptions)
    }

    /// Update a webhook subscription
    pub async fn update(
        &self,
        id: Uuid,
        input: UpdateWebhookSubscription,
    ) -> Result<Option<WebhookSubscription>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Connection(sqlx::Error::PoolClosed))?;

        let now = Utc::now();

        // Build dynamic update query
        let mut updates = vec!["updated_at = ?"];
        let mut has_name = false;
        let mut has_url = false;
        let mut has_secret = false;
        let mut has_events = false;
        let mut has_active = false;

        if input.name.is_some() {
            updates.push("name = ?");
            has_name = true;
        }
        if input.url.is_some() {
            updates.push("url = ?");
            has_url = true;
        }
        if input.secret.is_some() {
            updates.push("secret = ?");
            has_secret = true;
        }
        if input.events.is_some() {
            updates.push("events = ?");
            has_events = true;
        }
        if input.is_active.is_some() {
            updates.push("is_active = ?");
            has_active = true;
        }

        let query = crate::db::numbered_placeholders(&format!(
            "UPDATE webhook_subscriptions SET {} WHERE id = ?",
            updates.join(", ")
        ));

        let mut query_builder = sqlx::query(&query).bind(now.to_rfc3339());

        if has_name {
            query_builder = query_builder.bind(input.name.as_ref().unwrap());
        }
        if has_url {
            query_builder = query_builder.bind(input.url.as_ref().unwrap());
        }
        if has_secret {
            query_builder = query_builder.bind(input.secret.as_ref().unwrap());
        }
        if has_events {
            let events_json =
                serde_json::to_string(input.events.as_ref().unwrap()).map_err(|e| {
                    DatabaseError::Migration(format!("Failed to serialize events: {}", e))
                })?;
            query_builder = query_builder.bind(events_json);
        }
        if has_active {
            query_builder = query_builder.bind(if input.is_active.unwrap() { 1 } else { 0 });
        }

        query_builder = query_builder.bind(id.to_string());

        let result = query_builder.execute(pool).await?;

        if result.rows_affected() == 0 {
            return Ok(None);
        }

        self.get_by_id(id).await
    }

    /// Record a webhook trigger (success or failure)
    pub async fn record_trigger(&self, id: Uuid, success: bool) -> Result<(), DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Connection(sqlx::Error::PoolClosed))?;

        let now = Utc::now();

        if success {
            sqlx::query(
                r#"
                UPDATE webhook_subscriptions
                SET last_triggered_at = $1, failure_count = 0, updated_at = $2
                WHERE id = $3
                "#,
            )
            .bind(now.to_rfc3339())
            .bind(now.to_rfc3339())
            .bind(id.to_string())
            .execute(pool)
            .await?;
        } else {
            sqlx::query(
                r#"
                UPDATE webhook_subscriptions
                SET last_triggered_at = $1, failure_count = failure_count + 1, updated_at = $2
                WHERE id = $3
                "#,
            )
            .bind(now.to_rfc3339())
            .bind(now.to_rfc3339())
            .bind(id.to_string())
            .execute(pool)
            .await?;
        }

        Ok(())
    }

    /// Delete a webhook subscription
    pub async fn delete(&self, id: Uuid) -> Result<bool, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Connection(sqlx::Error::PoolClosed))?;

        let result = sqlx::query("DELETE FROM webhook_subscriptions WHERE id = $1")
            .bind(id.to_string())
            .execute(pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }
}

/// Database row for webhook subscriptions
#[derive(Debug, sqlx::FromRow)]
struct WebhookRow {
    id: String,
    org_id: String,
    name: String,
    url: String,
    secret: Option<String>,
    events: String,
    is_active: i32,
    last_triggered_at: Option<String>,
    failure_count: i32,
    created_at: String,
    updated_at: String,
}

impl TryFrom<WebhookRow> for WebhookSubscription {
    type Error = DatabaseError;

    fn try_from(row: WebhookRow) -> Result<Self, Self::Error> {
        let id = Uuid::parse_str(&row.id)
            .map_err(|e| DatabaseError::Migration(format!("Invalid UUID: {}", e)))?;
        let org_id = Uuid::parse_str(&row.org_id)
            .map_err(|e| DatabaseError::Migration(format!("Invalid org UUID: {}", e)))?;

        let events: Vec<WebhookEventType> = serde_json::from_str(&row.events)
            .map_err(|e| DatabaseError::Migration(format!("Invalid events JSON: {}", e)))?;

        let last_triggered_at = row
            .last_triggered_at
            .map(|s| chrono::DateTime::parse_from_rfc3339(&s))
            .transpose()
            .map_err(|e| DatabaseError::Migration(format!("Invalid timestamp: {}", e)))?
            .map(|dt| dt.with_timezone(&Utc));

        let created_at = chrono::DateTime::parse_from_rfc3339(&row.created_at)
            .map_err(|e| DatabaseError::Migration(format!("Invalid timestamp: {}", e)))?
            .with_timezone(&Utc);

        let updated_at = chrono::DateTime::parse_from_rfc3339(&row.updated_at)
            .map_err(|e| DatabaseError::Migration(format!("Invalid timestamp: {}", e)))?
            .with_timezone(&Utc);

        Ok(WebhookSubscription {
            id,
            org_id,
            name: row.name,
            url: row.url,
            secret: row.secret,
            events,
            is_active: row.is_active != 0,
            last_triggered_at,
            failure_count: row.failure_count,
            created_at,
            updated_at,
        })
    }
}
