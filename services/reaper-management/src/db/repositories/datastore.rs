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
    /// Position in the change stream at publish time — replicas loading
    /// this snapshot resume delta pulls from here.
    pub change_seq: i64,
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

    fn pool(&self) -> Result<&sqlx::AnyPool, DatabaseError> {
        self.db
            .any_pool()
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
               VALUES ($1, $2, $3, $4, $5, 0, $6, $7)"#,
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
               FROM datastores WHERE org_id = $1 AND namespace_id = $2"#,
        )
        .bind(org_id.to_string())
        .bind(namespace_id.to_string())
        .fetch_optional(pool)
        .await?;

        row.map(Self::row_to_record).transpose()
    }

    fn row_to_record(row: sqlx::any::AnyRow) -> Result<DatastoreRecord, DatabaseError> {
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
        sqlx::query("UPDATE datastores SET model = $1, updated_at = $2 WHERE id = $3")
            .bind(&model_json)
            .bind(Utc::now().to_rfc3339())
            .bind(datastore_id.to_string())
            .execute(pool)
            .await?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Change log (transactional outbox — D2 delta sync)
    // ------------------------------------------------------------------

    /// Append dirty-entity markers with freshly allocated monotonic
    /// sequence numbers, INSIDE the caller's transaction. This is what
    /// makes the outbox actually transactional: the mutation and its log
    /// entry commit or roll back TOGETHER — a crash between them cannot
    /// produce a change the replicas never hear about. One counter bump
    /// per batch keeps the save path lean regardless of fan-out.
    async fn record_changes_in(
        tx: &mut sqlx::Transaction<'_, sqlx::Any>,
        datastore_id: Uuid,
        marks: &[(String, bool)], // (entity_id, tombstone)
    ) -> Result<(), DatabaseError> {
        if marks.is_empty() {
            return Ok(());
        }
        let row = sqlx::query(
            "UPDATE datastores SET change_seq = change_seq + $1 WHERE id = $2 \
             RETURNING change_seq",
        )
        .bind(marks.len() as i64)
        .bind(datastore_id.to_string())
        .fetch_one(&mut **tx)
        .await?;
        let head: i64 = row.get("change_seq");
        let first = head - marks.len() as i64 + 1;
        let now = Utc::now().to_rfc3339();
        for (offset, (entity_id, tombstone)) in marks.iter().enumerate() {
            sqlx::query(
                "INSERT INTO adm_changes \
                 (id, datastore_id, seq, entity_id, tombstone, created_at) \
                 VALUES ($1, $2, $3, $4, $5, $6)",
            )
            .bind(Uuid::new_v4().to_string())
            .bind(datastore_id.to_string())
            .bind(first + offset as i64)
            .bind(entity_id)
            .bind(*tombstone as i64)
            .bind(&now)
            .execute(&mut **tx)
            .await?;
        }
        Ok(())
    }

    /// Dirty markers since `since_seq` (exclusive), deduplicated to the
    /// LATEST mark per entity (an entity churned 50 times syncs once).
    /// Returns (head_seq, min_available_seq, marks).
    pub async fn changes_since(
        &self,
        datastore_id: Uuid,
        since_seq: i64,
        limit: i64,
    ) -> Result<(i64, i64, Vec<(String, bool)>), DatabaseError> {
        let pool = self.pool()?;
        let head: i64 = sqlx::query("SELECT change_seq FROM datastores WHERE id = $1")
            .bind(datastore_id.to_string())
            .fetch_one(pool)
            .await?
            .get("change_seq");
        let min_available: i64 = sqlx::query(
            "SELECT COALESCE(MIN(seq), 0) AS min_seq FROM adm_changes WHERE datastore_id = $1",
        )
        .bind(datastore_id.to_string())
        .fetch_one(pool)
        .await?
        .get("min_seq");

        let rows = sqlx::query(
            "SELECT c.entity_id, c.seq AS last_seq, c.tombstone \
             FROM adm_changes c \
             JOIN (SELECT entity_id, MAX(seq) AS m FROM adm_changes \
                   WHERE datastore_id = $1 AND seq > $2 \
                   GROUP BY entity_id) latest \
               ON c.entity_id = latest.entity_id AND c.seq = latest.m \
             WHERE c.datastore_id = $1 \
             ORDER BY c.seq LIMIT $3",
        )
        .bind(datastore_id.to_string())
        .bind(since_seq)
        .bind(limit)
        .fetch_all(pool)
        .await?;
        let marks = rows
            .into_iter()
            .map(|row| {
                (
                    row.get::<String, _>("entity_id"),
                    row.get::<i64, _>("tombstone") != 0,
                )
            })
            .collect();
        Ok((head, min_available, marks))
    }

    /// Everything needed to materialize one entity's current doc: its
    /// record, its (unscoped) bindings, and every tuple touching it —
    /// three indexed point queries, never a dataset scan.
    pub async fn entity_view(
        &self,
        datastore_id: Uuid,
        entity_id: &str,
    ) -> Result<(Option<AdmEntity>, Vec<RoleBinding>, Vec<RelationTuple>), DatabaseError> {
        let entity = self.get_entity(datastore_id, entity_id).await?;
        let bindings = self
            .list_bindings(datastore_id, Some(entity_id), None)
            .await?;
        let pool = self.pool()?;
        let rows = sqlx::query(
            "SELECT object, relation, subject FROM adm_tuples              WHERE datastore_id = $1 AND (object = $2 OR subject = $3)              ORDER BY object, relation, subject",
        )
        .bind(datastore_id.to_string())
        .bind(entity_id)
        .bind(entity_id)
        .fetch_all(pool)
        .await?;
        let tuples = rows
            .into_iter()
            .map(|row| RelationTuple {
                object: row.get("object"),
                relation: row.get("relation"),
                subject: row.get("subject"),
            })
            .collect();
        Ok((entity, bindings, tuples))
    }

    /// REFERENTIAL CASCADE for entity deletion (the contract the
    /// delta==rebuild differential pinned): deleting an entity also deletes
    /// every tuple touching it and every binding it holds — a deleted group
    /// must not keep granting access as a dangling reference. Returns the
    /// OTHER endpoints of removed tuples (their docs changed too).
    pub async fn delete_entity_cascade(
        &self,
        datastore_id: Uuid,
        entity_id: &str,
    ) -> Result<(bool, Vec<String>), DatabaseError> {
        let pool = self.pool()?;
        let rows = sqlx::query(
            "SELECT object, subject FROM adm_tuples              WHERE datastore_id = $1 AND (object = $2 OR subject = $3)",
        )
        .bind(datastore_id.to_string())
        .bind(entity_id)
        .bind(entity_id)
        .fetch_all(pool)
        .await?;
        let mut affected: Vec<String> = Vec::new();
        for row in rows {
            for side in ["object", "subject"] {
                let id: String = row.get(side);
                if id != entity_id && !affected.contains(&id) {
                    affected.push(id);
                }
            }
        }
        let mut tx = pool.begin().await?;
        sqlx::query(
            "DELETE FROM adm_tuples WHERE datastore_id = $1 AND (object = $2 OR subject = $3)",
        )
        .bind(datastore_id.to_string())
        .bind(entity_id)
        .bind(entity_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM adm_role_bindings WHERE datastore_id = $1 AND subject = $2")
            .bind(datastore_id.to_string())
            .bind(entity_id)
            .execute(&mut *tx)
            .await?;
        let del =
            sqlx::query("DELETE FROM adm_entities WHERE datastore_id = $1 AND entity_id = $2")
                .bind(datastore_id.to_string())
                .bind(entity_id)
                .execute(&mut *tx)
                .await?;
        let deleted = del.rows_affected() > 0;

        let mut marks: Vec<(String, bool)> = vec![(entity_id.to_string(), true)];
        marks.extend(affected.iter().map(|id| (id.clone(), false)));
        Self::record_changes_in(&mut tx, datastore_id, &marks).await?;
        tx.commit().await?;
        Ok((deleted || !affected.is_empty(), affected))
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
        let mut tx = pool.begin().await?;
        sqlx::query(
            r#"INSERT INTO adm_entities
               (id, datastore_id, entity_id, entity_type, attributes, created_at, updated_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7)
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
        .execute(&mut *tx)
        .await?;
        Self::record_changes_in(&mut tx, datastore_id, &[(entity.entity_id.clone(), false)])
            .await?;
        tx.commit().await?;
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
             WHERE datastore_id = $1 AND entity_id = $2",
        )
        .bind(datastore_id.to_string())
        .bind(entity_id)
        .fetch_optional(pool)
        .await?;
        row.map(Self::row_to_entity).transpose()
    }

    fn row_to_entity(row: sqlx::any::AnyRow) -> Result<AdmEntity, DatabaseError> {
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
                     WHERE datastore_id = $1 AND entity_type = $2 ORDER BY entity_id",
                )
                .bind(datastore_id.to_string())
                .bind(t)
                .fetch_all(pool)
                .await?
            }
            None => {
                sqlx::query(
                    "SELECT entity_id, entity_type, attributes FROM adm_entities \
                     WHERE datastore_id = $1 ORDER BY entity_id",
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
            sqlx::query("DELETE FROM adm_entities WHERE datastore_id = $1 AND entity_id = $2")
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
        let mut tx = pool.begin().await?;
        sqlx::query(
            r#"INSERT INTO adm_role_bindings (id, datastore_id, subject, role, scope, created_at)
               VALUES ($1, $2, $3, $4, $5, $6)
               ON CONFLICT(datastore_id, subject, role, scope) DO NOTHING"#,
        )
        .bind(Uuid::new_v4().to_string())
        .bind(datastore_id.to_string())
        .bind(&binding.subject)
        .bind(&binding.role)
        .bind(&binding.scope)
        .bind(Utc::now().to_rfc3339())
        .execute(&mut *tx)
        .await?;
        Self::record_changes_in(&mut tx, datastore_id, &[(binding.subject.clone(), false)]).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn delete_binding(
        &self,
        datastore_id: Uuid,
        binding: &RoleBinding,
    ) -> Result<bool, DatabaseError> {
        let pool = self.pool()?;
        let mut tx = pool.begin().await?;
        let result = sqlx::query(
            "DELETE FROM adm_role_bindings \
             WHERE datastore_id = $1 AND subject = $2 AND role = $3 AND scope = $4",
        )
        .bind(datastore_id.to_string())
        .bind(&binding.subject)
        .bind(&binding.role)
        .bind(&binding.scope)
        .execute(&mut *tx)
        .await?;
        if result.rows_affected() > 0 {
            Self::record_changes_in(&mut tx, datastore_id, &[(binding.subject.clone(), false)])
                .await?;
        }
        tx.commit().await?;
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
        let sql = crate::db::numbered_placeholders(&sql);

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
        let mut tx = pool.begin().await?;
        sqlx::query(
            r#"INSERT INTO adm_tuples (id, datastore_id, object, relation, subject, created_at)
               VALUES ($1, $2, $3, $4, $5, $6)
               ON CONFLICT(datastore_id, object, relation, subject) DO NOTHING"#,
        )
        .bind(Uuid::new_v4().to_string())
        .bind(datastore_id.to_string())
        .bind(&tuple.object)
        .bind(&tuple.relation)
        .bind(&tuple.subject)
        .bind(Utc::now().to_rfc3339())
        .execute(&mut *tx)
        .await?;
        Self::record_changes_in(
            &mut tx,
            datastore_id,
            &[
                (tuple.object.clone(), false),
                (tuple.subject.clone(), false),
            ],
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn delete_tuple(
        &self,
        datastore_id: Uuid,
        tuple: &RelationTuple,
    ) -> Result<bool, DatabaseError> {
        let pool = self.pool()?;
        let mut tx = pool.begin().await?;
        let result = sqlx::query(
            "DELETE FROM adm_tuples \
             WHERE datastore_id = $1 AND object = $2 AND relation = $3 AND subject = $4",
        )
        .bind(datastore_id.to_string())
        .bind(&tuple.object)
        .bind(&tuple.relation)
        .bind(&tuple.subject)
        .execute(&mut *tx)
        .await?;
        if result.rows_affected() > 0 {
            Self::record_changes_in(
                &mut tx,
                datastore_id,
                &[
                    (tuple.object.clone(), false),
                    (tuple.subject.clone(), false),
                ],
            )
            .await?;
        }
        tx.commit().await?;
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
        let sql = crate::db::numbered_placeholders(&sql);

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

        // Pin the snapshot to its position in the change stream: agents
        // that load version N start pulling deltas from N's change_seq.
        let head_seq: i64 = sqlx::query("SELECT change_seq FROM datastores WHERE id = $1")
            .bind(store.id.to_string())
            .fetch_one(pool)
            .await?
            .get("change_seq");

        sqlx::query(
            r#"INSERT INTO adm_versions
               (id, datastore_id, version, checksum, document,
                entity_count, tuple_count, binding_count, published_by, published_at,
                change_seq)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)"#,
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
        .bind(head_seq)
        .execute(pool)
        .await?;

        // Compact the outbox behind the PREVIOUS snapshot: any replica older
        // than that must full-sync anyway (changes_since reports the floor,
        // the API tells such replicas snapshot_required).
        if store.current_version > 0 {
            if let Some(prev) = sqlx::query(
                "SELECT change_seq FROM adm_versions                  WHERE datastore_id = $1 AND version = $2",
            )
            .bind(store.id.to_string())
            .bind(store.current_version)
            .fetch_optional(pool)
            .await?
            {
                let prev_seq: i64 = prev.get("change_seq");
                sqlx::query("DELETE FROM adm_changes WHERE datastore_id = $1 AND seq <= $2")
                    .bind(store.id.to_string())
                    .bind(prev_seq)
                    .execute(pool)
                    .await?;
            }
        }

        sqlx::query("UPDATE datastores SET current_version = $1, updated_at = $2 WHERE id = $3")
            .bind(version)
            .bind(&now)
            .bind(store.id.to_string())
            .execute(pool)
            .await?;

        Ok(PublishedVersion {
            version,
            checksum,
            change_seq: head_seq,
            entity_count: entities.len() as i64,
            tuple_count: tuples.len() as i64,
            binding_count: bindings.len() as i64,
            published_by: published_by.to_string(),
            published_at: now,
        })
    }

    /// Record counts via COUNT(*) — the status endpoint must not pay for
    /// materializing every row just to show three numbers.
    pub async fn counts(&self, datastore_id: Uuid) -> Result<(i64, i64, i64), DatabaseError> {
        let pool = self.pool()?;
        let id = datastore_id.to_string();
        let row = sqlx::query(
            "SELECT \
               (SELECT COUNT(*) FROM adm_entities WHERE datastore_id = $1) AS entities, \
               (SELECT COUNT(*) FROM adm_role_bindings WHERE datastore_id = $1) AS bindings, \
               (SELECT COUNT(*) FROM adm_tuples WHERE datastore_id = $1) AS tuples",
        )
        .bind(&id)
        .fetch_one(pool)
        .await?;
        Ok((row.get("entities"), row.get("bindings"), row.get("tuples")))
    }

    pub async fn list_versions(
        &self,
        datastore_id: Uuid,
    ) -> Result<Vec<PublishedVersion>, DatabaseError> {
        let pool = self.pool()?;
        let rows = sqlx::query(
            "SELECT version, checksum, change_seq, entity_count, tuple_count, binding_count, \
                    published_by, published_at \
             FROM adm_versions WHERE datastore_id = $1 ORDER BY version DESC",
        )
        .bind(datastore_id.to_string())
        .fetch_all(pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| PublishedVersion {
                version: row.get("version"),
                checksum: row.get("checksum"),
                change_seq: row.get("change_seq"),
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
            "SELECT version, checksum, change_seq, document, entity_count, tuple_count, binding_count, \
                    published_by, published_at \
             FROM adm_versions WHERE datastore_id = $1 AND version = $2",
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
                    change_seq: row.get("change_seq"),
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
