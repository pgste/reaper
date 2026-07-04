//! Datastore repository — persistence for the Authorization Data Model.
//!
//! One datastore per (org, namespace). Records (entities, role bindings,
//! relationship tuples) are validated against the model at the API layer;
//! this layer owns storage, versioning, and publish materialization.

use chrono::Utc;
use sha2::{Digest, Sha256};
use sqlx::Row;
use uuid::Uuid;

use crate::db::{Database, DatabaseError};
use crate::domain::datastore::{
    materialize, AdmEntity, DatastoreTemplate, ModelDefinition, RelationTuple, RoleBinding,
};

/// A provisioned datastore row.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DatastoreRecord {
    pub id: Uuid,
    pub org_id: Uuid,
    pub namespace_id: Uuid,
    pub template: String,
    pub model: ModelDefinition,
    pub current_version: i64,
    pub created_at: String,
    pub updated_at: String,
}

/// A published, immutable data-bundle version.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PublishedVersion {
    pub version: i64,
    pub checksum: String,
    pub entity_count: i64,
    pub tuple_count: i64,
    pub binding_count: i64,
    pub published_by: String,
    pub published_at: String,
}

pub struct DatastoreRepository<'a> {
    db: &'a Database,
}

impl<'a> DatastoreRepository<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    fn pool(&self) -> Result<&sqlx::SqlitePool, DatabaseError> {
        self.db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))
    }

    // ------------------------------------------------------------------
    // Datastore lifecycle
    // ------------------------------------------------------------------

    pub async fn provision(
        &self,
        org_id: Uuid,
        namespace_id: Uuid,
        template: DatastoreTemplate,
    ) -> Result<DatastoreRecord, DatabaseError> {
        let pool = self.pool()?;
        let id = Uuid::new_v4();
        let now = Utc::now().to_rfc3339();
        let model = template.seed_model();
        let model_json = serde_json::to_string(&model)
            .map_err(|e| DatabaseError::Config(format!("serialize model: {e}")))?;

        sqlx::query(
            r#"INSERT INTO datastores
               (id, org_id, namespace_id, template, model, current_version, created_at, updated_at)
               VALUES (?, ?, ?, ?, ?, 0, ?, ?)"#,
        )
        .bind(id.to_string())
        .bind(org_id.to_string())
        .bind(namespace_id.to_string())
        .bind(template.as_str())
        .bind(&model_json)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await?;

        Ok(DatastoreRecord {
            id,
            org_id,
            namespace_id,
            template: template.as_str().to_string(),
            model,
            current_version: 0,
            created_at: now.clone(),
            updated_at: now,
        })
    }

    pub async fn get(
        &self,
        org_id: Uuid,
        namespace_id: Uuid,
    ) -> Result<Option<DatastoreRecord>, DatabaseError> {
        let pool = self.pool()?;
        let row = sqlx::query(
            r#"SELECT id, org_id, namespace_id, template, model, current_version,
                      created_at, updated_at
               FROM datastores WHERE org_id = ? AND namespace_id = ?"#,
        )
        .bind(org_id.to_string())
        .bind(namespace_id.to_string())
        .fetch_optional(pool)
        .await?;

        row.map(Self::row_to_record).transpose()
    }

    fn row_to_record(row: sqlx::sqlite::SqliteRow) -> Result<DatastoreRecord, DatabaseError> {
        let model: ModelDefinition =
            serde_json::from_str(row.get::<String, _>("model").as_str())
                .map_err(|e| DatabaseError::Config(format!("corrupt model json: {e}")))?;
        let parse = |field: &str| -> Result<Uuid, DatabaseError> {
            Uuid::parse_str(row.get::<String, _>(field).as_str())
                .map_err(|e| DatabaseError::Config(format!("corrupt uuid in {field}: {e}")))
        };
        Ok(DatastoreRecord {
            id: parse("id")?,
            org_id: parse("org_id")?,
            namespace_id: parse("namespace_id")?,
            template: row.get("template"),
            model,
            current_version: row.get("current_version"),
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
        })
    }

    pub async fn update_model(
        &self,
        datastore_id: Uuid,
        model: &ModelDefinition,
    ) -> Result<(), DatabaseError> {
        let pool = self.pool()?;
        let model_json = serde_json::to_string(model)
            .map_err(|e| DatabaseError::Config(format!("serialize model: {e}")))?;
        sqlx::query("UPDATE datastores SET model = ?, updated_at = ? WHERE id = ?")
            .bind(&model_json)
            .bind(Utc::now().to_rfc3339())
            .bind(datastore_id.to_string())
            .execute(pool)
            .await?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Entities
    // ------------------------------------------------------------------

    pub async fn upsert_entity(
        &self,
        datastore_id: Uuid,
        entity: &AdmEntity,
    ) -> Result<(), DatabaseError> {
        let pool = self.pool()?;
        let now = Utc::now().to_rfc3339();
        let attrs = serde_json::to_string(&entity.attributes)
            .map_err(|e| DatabaseError::Config(format!("serialize attributes: {e}")))?;
        sqlx::query(
            r#"INSERT INTO adm_entities
               (id, datastore_id, entity_id, entity_type, attributes, created_at, updated_at)
               VALUES (?, ?, ?, ?, ?, ?, ?)
               ON CONFLICT(datastore_id, entity_id)
               DO UPDATE SET entity_type = excluded.entity_type,
                             attributes = excluded.attributes,
                             updated_at = excluded.updated_at"#,
        )
        .bind(Uuid::new_v4().to_string())
        .bind(datastore_id.to_string())
        .bind(&entity.entity_id)
        .bind(&entity.entity_type)
        .bind(&attrs)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await?;
        Ok(())
    }

    pub async fn get_entity(
        &self,
        datastore_id: Uuid,
        entity_id: &str,
    ) -> Result<Option<AdmEntity>, DatabaseError> {
        let pool = self.pool()?;
        let row = sqlx::query(
            "SELECT entity_id, entity_type, attributes FROM adm_entities \
             WHERE datastore_id = ? AND entity_id = ?",
        )
        .bind(datastore_id.to_string())
        .bind(entity_id)
        .fetch_optional(pool)
        .await?;
        row.map(Self::row_to_entity).transpose()
    }

    fn row_to_entity(row: sqlx::sqlite::SqliteRow) -> Result<AdmEntity, DatabaseError> {
        let attributes = serde_json::from_str(row.get::<String, _>("attributes").as_str())
            .map_err(|e| DatabaseError::Config(format!("corrupt attributes json: {e}")))?;
        Ok(AdmEntity {
            entity_id: row.get("entity_id"),
            entity_type: row.get("entity_type"),
            attributes,
        })
    }

    pub async fn list_entities(
        &self,
        datastore_id: Uuid,
        entity_type: Option<&str>,
    ) -> Result<Vec<AdmEntity>, DatabaseError> {
        let pool = self.pool()?;
        let rows = match entity_type {
            Some(t) => {
                sqlx::query(
                    "SELECT entity_id, entity_type, attributes FROM adm_entities \
                     WHERE datastore_id = ? AND entity_type = ? ORDER BY entity_id",
                )
                .bind(datastore_id.to_string())
                .bind(t)
                .fetch_all(pool)
                .await?
            }
            None => {
                sqlx::query(
                    "SELECT entity_id, entity_type, attributes FROM adm_entities \
                     WHERE datastore_id = ? ORDER BY entity_id",
                )
                .bind(datastore_id.to_string())
                .fetch_all(pool)
                .await?
            }
        };
        rows.into_iter().map(Self::row_to_entity).collect()
    }

    pub async fn delete_entity(
        &self,
        datastore_id: Uuid,
        entity_id: &str,
    ) -> Result<bool, DatabaseError> {
        let pool = self.pool()?;
        let result =
            sqlx::query("DELETE FROM adm_entities WHERE datastore_id = ? AND entity_id = ?")
                .bind(datastore_id.to_string())
                .bind(entity_id)
                .execute(pool)
                .await?;
        Ok(result.rows_affected() > 0)
    }

    // ------------------------------------------------------------------
    // Role bindings
    // ------------------------------------------------------------------

    pub async fn add_binding(
        &self,
        datastore_id: Uuid,
        binding: &RoleBinding,
    ) -> Result<(), DatabaseError> {
        let pool = self.pool()?;
        sqlx::query(
            r#"INSERT INTO adm_role_bindings (id, datastore_id, subject, role, scope, created_at)
               VALUES (?, ?, ?, ?, ?, ?)
               ON CONFLICT(datastore_id, subject, role, scope) DO NOTHING"#,
        )
        .bind(Uuid::new_v4().to_string())
        .bind(datastore_id.to_string())
        .bind(&binding.subject)
        .bind(&binding.role)
        .bind(&binding.scope)
        .bind(Utc::now().to_rfc3339())
        .execute(pool)
        .await?;
        Ok(())
    }

    pub async fn delete_binding(
        &self,
        datastore_id: Uuid,
        binding: &RoleBinding,
    ) -> Result<bool, DatabaseError> {
        let pool = self.pool()?;
        let result = sqlx::query(
            "DELETE FROM adm_role_bindings \
             WHERE datastore_id = ? AND subject = ? AND role = ? AND scope = ?",
        )
        .bind(datastore_id.to_string())
        .bind(&binding.subject)
        .bind(&binding.role)
        .bind(&binding.scope)
        .execute(pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn list_bindings(
        &self,
        datastore_id: Uuid,
        subject: Option<&str>,
        role: Option<&str>,
    ) -> Result<Vec<RoleBinding>, DatabaseError> {
        let pool = self.pool()?;
        let mut sql = String::from(
            "SELECT subject, role, scope FROM adm_role_bindings WHERE datastore_id = ?",
        );
        if subject.is_some() {
            sql.push_str(" AND subject = ?");
        }
        if role.is_some() {
            sql.push_str(" AND role = ?");
        }
        sql.push_str(" ORDER BY subject, role, scope");

        let mut query = sqlx::query(&sql).bind(datastore_id.to_string());
        if let Some(s) = subject {
            query = query.bind(s);
        }
        if let Some(r) = role {
            query = query.bind(r);
        }
        let rows = query.fetch_all(pool).await?;
        Ok(rows
            .into_iter()
            .map(|row| RoleBinding {
                subject: row.get("subject"),
                role: row.get("role"),
                scope: row.get("scope"),
            })
            .collect())
    }

    // ------------------------------------------------------------------
    // Relationship tuples
    // ------------------------------------------------------------------

    pub async fn write_tuple(
        &self,
        datastore_id: Uuid,
        tuple: &RelationTuple,
    ) -> Result<(), DatabaseError> {
        let pool = self.pool()?;
        sqlx::query(
            r#"INSERT INTO adm_tuples (id, datastore_id, object, relation, subject, created_at)
               VALUES (?, ?, ?, ?, ?, ?)
               ON CONFLICT(datastore_id, object, relation, subject) DO NOTHING"#,
        )
        .bind(Uuid::new_v4().to_string())
        .bind(datastore_id.to_string())
        .bind(&tuple.object)
        .bind(&tuple.relation)
        .bind(&tuple.subject)
        .bind(Utc::now().to_rfc3339())
        .execute(pool)
        .await?;
        Ok(())
    }

    pub async fn delete_tuple(
        &self,
        datastore_id: Uuid,
        tuple: &RelationTuple,
    ) -> Result<bool, DatabaseError> {
        let pool = self.pool()?;
        let result = sqlx::query(
            "DELETE FROM adm_tuples \
             WHERE datastore_id = ? AND object = ? AND relation = ? AND subject = ?",
        )
        .bind(datastore_id.to_string())
        .bind(&tuple.object)
        .bind(&tuple.relation)
        .bind(&tuple.subject)
        .execute(pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn list_tuples(
        &self,
        datastore_id: Uuid,
        object: Option<&str>,
        relation: Option<&str>,
        subject: Option<&str>,
    ) -> Result<Vec<RelationTuple>, DatabaseError> {
        let pool = self.pool()?;
        let mut sql =
            String::from("SELECT object, relation, subject FROM adm_tuples WHERE datastore_id = ?");
        if object.is_some() {
            sql.push_str(" AND object = ?");
        }
        if relation.is_some() {
            sql.push_str(" AND relation = ?");
        }
        if subject.is_some() {
            sql.push_str(" AND subject = ?");
        }
        sql.push_str(" ORDER BY object, relation, subject");

        let mut query = sqlx::query(&sql).bind(datastore_id.to_string());
        for value in [object, relation, subject].into_iter().flatten() {
            query = query.bind(value);
        }
        let rows = query.fetch_all(pool).await?;
        Ok(rows
            .into_iter()
            .map(|row| RelationTuple {
                object: row.get("object"),
                relation: row.get("relation"),
                subject: row.get("subject"),
            })
            .collect())
    }

    // ------------------------------------------------------------------
    // Publish + versions
    // ------------------------------------------------------------------

    /// Materialize current records into an immutable data-bundle version.
    /// Returns the new version row (the document itself is fetched via
    /// `get_version`).
    pub async fn publish(
        &self,
        store: &DatastoreRecord,
        published_by: &str,
    ) -> Result<PublishedVersion, DatabaseError> {
        let pool = self.pool()?;

        let entities = self.list_entities(store.id, None).await?;
        let bindings = self.list_bindings(store.id, None, None).await?;
        let tuples = self.list_tuples(store.id, None, None, None).await?;

        let document = materialize(&store.model, &entities, &bindings, &tuples);
        let document_json = serde_json::to_string(&document)
            .map_err(|e| DatabaseError::Config(format!("serialize document: {e}")))?;
        let checksum = format!("sha256:{:x}", Sha256::digest(document_json.as_bytes()));

        let version = store.current_version + 1;
        let now = Utc::now().to_rfc3339();

        sqlx::query(
            r#"INSERT INTO adm_versions
               (id, datastore_id, version, checksum, document,
                entity_count, tuple_count, binding_count, published_by, published_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
        )
        .bind(Uuid::new_v4().to_string())
        .bind(store.id.to_string())
        .bind(version)
        .bind(&checksum)
        .bind(&document_json)
        .bind(entities.len() as i64)
        .bind(tuples.len() as i64)
        .bind(bindings.len() as i64)
        .bind(published_by)
        .bind(&now)
        .execute(pool)
        .await?;

        sqlx::query("UPDATE datastores SET current_version = ?, updated_at = ? WHERE id = ?")
            .bind(version)
            .bind(&now)
            .bind(store.id.to_string())
            .execute(pool)
            .await?;

        Ok(PublishedVersion {
            version,
            checksum,
            entity_count: entities.len() as i64,
            tuple_count: tuples.len() as i64,
            binding_count: bindings.len() as i64,
            published_by: published_by.to_string(),
            published_at: now,
        })
    }

    pub async fn list_versions(
        &self,
        datastore_id: Uuid,
    ) -> Result<Vec<PublishedVersion>, DatabaseError> {
        let pool = self.pool()?;
        let rows = sqlx::query(
            "SELECT version, checksum, entity_count, tuple_count, binding_count, \
                    published_by, published_at \
             FROM adm_versions WHERE datastore_id = ? ORDER BY version DESC",
        )
        .bind(datastore_id.to_string())
        .fetch_all(pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| PublishedVersion {
                version: row.get("version"),
                checksum: row.get("checksum"),
                entity_count: row.get("entity_count"),
                tuple_count: row.get("tuple_count"),
                binding_count: row.get("binding_count"),
                published_by: row.get("published_by"),
                published_at: row.get("published_at"),
            })
            .collect())
    }

    /// Fetch a published version's materialized document (what agents load).
    pub async fn get_version_document(
        &self,
        datastore_id: Uuid,
        version: i64,
    ) -> Result<Option<(PublishedVersion, String)>, DatabaseError> {
        let pool = self.pool()?;
        let row = sqlx::query(
            "SELECT version, checksum, document, entity_count, tuple_count, binding_count, \
                    published_by, published_at \
             FROM adm_versions WHERE datastore_id = ? AND version = ?",
        )
        .bind(datastore_id.to_string())
        .bind(version)
        .fetch_optional(pool)
        .await?;
        Ok(row.map(|row| {
            (
                PublishedVersion {
                    version: row.get("version"),
                    checksum: row.get("checksum"),
                    entity_count: row.get("entity_count"),
                    tuple_count: row.get("tuple_count"),
                    binding_count: row.get("binding_count"),
                    published_by: row.get("published_by"),
                    published_at: row.get("published_at"),
                },
                row.get::<String, _>("document"),
            )
        }))
    }
}
