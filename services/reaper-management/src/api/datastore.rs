//! Data-plane API — managed Authorization Data Model per namespace.
//!
//! Provision a datastore from a template (rbac/abac/rebac/combined), manage
//! its records through typed, schema-validated CRUD (entities/attributes,
//! role bindings, relationship tuples), then `publish` to cut an immutable,
//! checksummed data-bundle version that agents load. Every surface is plain
//! REST under org auth + API keys, so customers can build their own tooling
//! on top. See docs/development/DATA_PLANE_PLAN.md (Phase D1).

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{Json, Response},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    api::error::{ApiError, ApiResult, ProblemDetails},
    api::idempotency,
    api::pagination::{PageQuery, Paginated},
    auth::{middleware::AuthenticatedUser, middleware::RequireAuth, scopes::Scope},
    db::repositories::datastore::{DatastoreRecord, PageRow, PublishedVersion},
    db::repositories::{DatastoreRepository, NamespaceRepository},
    db::DatabaseError,
    domain::datastore::{
        materialize, AdmEntity, DatastoreTemplate, ModelDefinition, RelationTuple, RoleBinding,
    },
    domain::{impact, migration},
    state::{AppState, ServerEvent},
};

pub fn routes() -> OpenApiRouter<Arc<AppState>> {
    OpenApiRouter::new()
        .routes(routes!(get_datastore, provision))
        .routes(routes!(get_model, put_model))
        .routes(routes!(list_entities, upsert_entity))
        .routes(routes!(get_entity, delete_entity))
        .routes(routes!(put_attributes))
        .routes(routes!(list_bindings, add_binding, remove_binding))
        .routes(routes!(list_tuples, write_tuple, remove_tuple))
        .routes(routes!(publish))
        .routes(routes!(plan_migration))
        .routes(routes!(apply_migration))
        .routes(routes!(list_migrations))
        .routes(routes!(rollback_migration))
        .routes(routes!(get_changes))
        .routes(routes!(list_versions))
        .routes(routes!(get_version))
}

// ---------------------------------------------------------------------------
// Auth + resolution
// ---------------------------------------------------------------------------

struct Resolved {
    org_id: Uuid,
    namespace_id: Uuid,
}

/// Reads need agent:read (agents/sync fetch versions); writes need
/// org admin or agent:write (automation API keys driving data).
async fn authorize(
    state: &AppState,
    user: &AuthenticatedUser,
    org_ref: &str,
    ns_slug: &str,
    write: bool,
) -> ApiResult<Resolved> {
    let required: &[Scope] = if write {
        &[Scope::OrgAdmin, Scope::AgentWrite]
    } else {
        &[Scope::AgentRead, Scope::OrgAdmin]
    };
    let organization = crate::api::orgs::authorize_org(state, user, org_ref, required).await?;

    let ns_repo = NamespaceRepository::new(&state.db);
    let namespace = ns_repo
        .get_by_slug(organization.id, ns_slug)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("namespace '{ns_slug}' not found")))?;

    Ok(Resolved {
        org_id: organization.id,
        namespace_id: namespace.id,
    })
}

/// Wake the fleet after a publish: agents/sync on THIS instance via the
/// broadcast stream, sibling management instances via pg_notify (no-op on
/// SQLite) so their connected agents wake too.
async fn notify_published(
    state: &AppState,
    resolved: &Resolved,
    datastore_id: Uuid,
    published: &crate::db::repositories::datastore::PublishedVersion,
) {
    let _ = state.event_tx.send(ServerEvent::DatastorePublished {
        datastore_id,
        org_id: resolved.org_id,
        namespace_id: Some(resolved.namespace_id),
        version: published.version,
        checksum: published.checksum.clone(),
    });
    crate::events_pg::notify_datastore_published(
        state,
        datastore_id,
        resolved.org_id,
        Some(resolved.namespace_id),
        published.version,
        &published.checksum,
    )
    .await;
}

async fn require_store(state: &AppState, resolved: &Resolved) -> ApiResult<DatastoreRecord> {
    DatastoreRepository::new(&state.db)
        .get(resolved.org_id, resolved.namespace_id)
        .await?
        .ok_or_else(|| {
            ApiError::NotFound(
                "no datastore provisioned for this namespace (POST …/datastore with a template)"
                    .to_string(),
            )
        })
}

// ---------------------------------------------------------------------------
// Datastore lifecycle
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, ToSchema)]
struct ProvisionRequest {
    /// Starter model: `rbac`, `abac`, `rebac`, or `combined`.
    template: DatastoreTemplate,
}

/// The freshly provisioned datastore.
#[derive(Debug, Serialize, ToSchema)]
struct ProvisionResponse {
    id: Uuid,
    /// The template the datastore was provisioned from.
    template: String,
    model: ModelDefinition,
    /// Latest published data-bundle version (0 = never published).
    current_version: i64,
}

/// Provision the namespace's datastore from a template (one per namespace).
#[utoipa::path(
    post,
    path = "/orgs/{org}/namespaces/{ns}/datastore",
    tag = "datastore",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("ns" = String, Path, description = "Namespace slug")
    ),
    request_body = ProvisionRequest,
    responses(
        (status = 200, description = "Datastore provisioned", body = ProvisionResponse),
        (status = 404, description = "Organization or namespace not found", body = ProblemDetails),
        (status = 409, description = "Datastore already provisioned", body = ProblemDetails)
    ),
    security(("bearer_jwt" = []))
)]
async fn provision(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, ns)): Path<(String, String)>,
    Json(req): Json<ProvisionRequest>,
) -> ApiResult<Json<ProvisionResponse>> {
    let resolved = authorize(&state, &user, &org, &ns, true).await?;
    let repo = DatastoreRepository::new(&state.db);
    if repo
        .get(resolved.org_id, resolved.namespace_id)
        .await?
        .is_some()
    {
        return Err(ApiError::Conflict(
            "datastore already provisioned for this namespace".to_string(),
        ));
    }
    let record = repo
        .provision(resolved.org_id, resolved.namespace_id, req.template)
        .await?;
    Ok(Json(ProvisionResponse {
        id: record.id,
        template: record.template,
        model: record.model,
        current_version: record.current_version,
    }))
}

/// Record counts per kind.
#[derive(Debug, Serialize, ToSchema)]
struct RecordCounts {
    entities: i64,
    role_bindings: i64,
    tuples: i64,
}

/// Datastore summary: template, publish state, record counts.
#[derive(Debug, Serialize, ToSchema)]
struct DatastoreSummaryResponse {
    id: Uuid,
    template: String,
    /// Latest published data-bundle version (0 = never published).
    current_version: i64,
    counts: RecordCounts,
    created_at: String,
    updated_at: String,
}

/// Datastore summary: template, publish state, record counts.
#[utoipa::path(
    get,
    path = "/orgs/{org}/namespaces/{ns}/datastore",
    tag = "datastore",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("ns" = String, Path, description = "Namespace slug")
    ),
    responses(
        (status = 200, description = "Datastore summary", body = DatastoreSummaryResponse),
        (status = 404, description = "No datastore provisioned for this namespace", body = ProblemDetails)
    ),
    security(("bearer_jwt" = []))
)]
async fn get_datastore(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, ns)): Path<(String, String)>,
) -> ApiResult<Json<DatastoreSummaryResponse>> {
    let resolved = authorize(&state, &user, &org, &ns, false).await?;
    let store = require_store(&state, &resolved).await?;
    let (entities, bindings, tuples) = DatastoreRepository::new(&state.db).counts(store.id).await?;
    Ok(Json(DatastoreSummaryResponse {
        id: store.id,
        template: store.template,
        current_version: store.current_version,
        counts: RecordCounts {
            entities,
            role_bindings: bindings,
            tuples,
        },
        created_at: store.created_at,
        updated_at: store.updated_at,
    }))
}

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

/// Fetch the datastore's model definition (entity types, roles, relations).
#[utoipa::path(
    get,
    path = "/orgs/{org}/namespaces/{ns}/datastore/model",
    tag = "datastore",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("ns" = String, Path, description = "Namespace slug")
    ),
    responses(
        (status = 200, description = "Datastore model definition", body = ModelDefinition),
        (status = 404, description = "No datastore provisioned for this namespace", body = ProblemDetails)
    ),
    security(("bearer_jwt" = []))
)]
async fn get_model(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, ns)): Path<(String, String)>,
) -> ApiResult<Json<ModelDefinition>> {
    let resolved = authorize(&state, &user, &org, &ns, false).await?;
    let store = require_store(&state, &resolved).await?;
    Ok(Json(store.model))
}

/// Acknowledgement of an additive model overwrite.
#[derive(Debug, Serialize, ToSchema)]
struct ModelUpdatedResponse {
    updated: bool,
}

/// Overwrite the model (additive edits only; breaking changes need a migration).
#[utoipa::path(
    put,
    path = "/orgs/{org}/namespaces/{ns}/datastore/model",
    tag = "datastore",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("ns" = String, Path, description = "Namespace slug")
    ),
    request_body = ModelDefinition,
    responses(
        (status = 200, description = "Model updated", body = ModelUpdatedResponse),
        (status = 404, description = "No datastore provisioned for this namespace", body = ProblemDetails),
        (status = 409, description = "Overwrite would break existing vocabulary — use a migration", body = ProblemDetails)
    ),
    security(("bearer_jwt" = []))
)]
async fn put_model(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, ns)): Path<(String, String)>,
    Json(model): Json<ModelDefinition>,
) -> ApiResult<Json<ModelUpdatedResponse>> {
    let resolved = authorize(&state, &user, &org, &ns, true).await?;
    let store = require_store(&state, &resolved).await?;
    // A bare overwrite that changes vocabulary would silently strand every
    // record still using the old names/types — the exact hazard the
    // migration engine exists to prevent (Plan 12 step 4). Additive edits
    // pass; renames/removals/retypes must go through …/migrations/plan +
    // apply, which transform the records atomically alongside the model.
    let breaks = migration::vocabulary_breaking_changes(&store.model, &model);
    if !breaks.is_empty() {
        return Err(ApiError::Conflict(format!(
            "model overwrite would break existing vocabulary ({}); use \
             POST …/datastore/migrations/plan + /apply so records are \
             transformed with the model",
            breaks.join("; ")
        )));
    }
    DatastoreRepository::new(&state.db)
        .update_model(store.id, &model)
        .await?;
    Ok(Json(ModelUpdatedResponse { updated: true }))
}

// ---------------------------------------------------------------------------
// Entities
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct EntityListParams {
    #[serde(rename = "type")]
    entity_type: Option<String>,
    limit: Option<i64>,
    cursor: Option<String>,
}

/// List entities, keyset-paginated (Plan 07 Phase E; closes R2-01 — the entity
/// table is the biggest table in a real deployment and must never be listed
/// unbounded).
#[utoipa::path(
    get,
    path = "/orgs/{org}/namespaces/{ns}/datastore/entities",
    tag = "datastore",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("ns" = String, Path, description = "Namespace slug"),
        ("type" = Option<String>, Query, description = "Filter by entity type"),
        ("limit" = Option<i64>, Query, description = "Page size (default 50, max 200)"),
        ("cursor" = Option<String>, Query, description = "Opaque cursor from the previous page's next_cursor")
    ),
    responses(
        (status = 200, description = "One page of entities with a next_cursor to resume",
         body = Paginated<AdmEntity>),
        (status = 400, description = "limit out of range or cursor invalid", body = ProblemDetails),
        (status = 404, description = "No datastore provisioned for this namespace", body = ProblemDetails)
    ),
    security(("bearer_jwt" = []))
)]
async fn list_entities(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, ns)): Path<(String, String)>,
    Query(params): Query<EntityListParams>,
) -> ApiResult<Json<Paginated<PageRow<AdmEntity>>>> {
    let resolved = authorize(&state, &user, &org, &ns, false).await?;
    let store = require_store(&state, &resolved).await?;
    let page = PageQuery {
        limit: params.limit,
        cursor: params.cursor,
    }
    .validate()?;
    let rows = DatastoreRepository::new(&state.db)
        .list_entities_page(
            store.id,
            params.entity_type.as_deref(),
            page.limit + 1,
            page.after.as_ref(),
        )
        .await?;
    Ok(Json(Paginated::from_rows(rows, &page, PageRow::key)))
}

/// Acknowledgement of an entity upsert.
#[derive(Debug, Serialize, ToSchema)]
struct EntityUpsertedResponse {
    entity_id: String,
    upserted: bool,
}

/// Create or replace an entity (attributes validated against the model).
#[utoipa::path(
    post,
    path = "/orgs/{org}/namespaces/{ns}/datastore/entities",
    tag = "datastore",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("ns" = String, Path, description = "Namespace slug")
    ),
    request_body = AdmEntity,
    responses(
        (status = 200, description = "Entity upserted", body = EntityUpsertedResponse),
        (status = 400, description = "Attributes violate the model", body = ProblemDetails),
        (status = 404, description = "No datastore provisioned for this namespace", body = ProblemDetails)
    ),
    security(("bearer_jwt" = []))
)]
async fn upsert_entity(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, ns)): Path<(String, String)>,
    Json(entity): Json<AdmEntity>,
) -> ApiResult<Json<EntityUpsertedResponse>> {
    let resolved = authorize(&state, &user, &org, &ns, true).await?;
    let store = require_store(&state, &resolved).await?;
    store
        .model
        .validate_attributes(&entity.entity_type, &entity.attributes)
        .map_err(ApiError::BadRequest)?;
    DatastoreRepository::new(&state.db)
        .upsert_entity(store.id, &entity)
        .await?;
    Ok(Json(EntityUpsertedResponse {
        entity_id: entity.entity_id,
        upserted: true,
    }))
}

/// Fetch one entity with its typed attributes.
#[utoipa::path(
    get,
    path = "/orgs/{org}/namespaces/{ns}/datastore/entities/{entity_id}",
    tag = "datastore",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("ns" = String, Path, description = "Namespace slug"),
        ("entity_id" = String, Path, description = "Entity ID")
    ),
    responses(
        (status = 200, description = "Entity detail", body = AdmEntity),
        (status = 404, description = "Entity (or datastore) not found", body = ProblemDetails)
    ),
    security(("bearer_jwt" = []))
)]
async fn get_entity(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, ns, entity_id)): Path<(String, String, String)>,
) -> ApiResult<Json<AdmEntity>> {
    let resolved = authorize(&state, &user, &org, &ns, false).await?;
    let store = require_store(&state, &resolved).await?;
    let entity = DatastoreRepository::new(&state.db)
        .get_entity(store.id, &entity_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("entity '{entity_id}' not found")))?;
    Ok(Json(entity))
}

/// Result of an entity delete, including the referential cascade.
#[derive(Debug, Serialize, ToSchema)]
struct EntityDeletedResponse {
    /// Whether the entity record existed.
    deleted: bool,
    /// Other entity ids whose materialized documents changed because tuples
    /// or bindings touching the deleted entity died with it.
    cascaded: Vec<String>,
}

/// Delete an entity; tuples and bindings touching it cascade.
#[utoipa::path(
    delete,
    path = "/orgs/{org}/namespaces/{ns}/datastore/entities/{entity_id}",
    tag = "datastore",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("ns" = String, Path, description = "Namespace slug"),
        ("entity_id" = String, Path, description = "Entity ID")
    ),
    responses(
        (status = 200, description = "Entity deleted (with cascade)", body = EntityDeletedResponse),
        (status = 404, description = "No datastore provisioned for this namespace", body = ProblemDetails)
    ),
    security(("bearer_jwt" = []))
)]
async fn delete_entity(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, ns, entity_id)): Path<(String, String, String)>,
) -> ApiResult<Json<EntityDeletedResponse>> {
    let resolved = authorize(&state, &user, &org, &ns, true).await?;
    let store = require_store(&state, &resolved).await?;
    // Referential cascade (contract pinned by the delta==rebuild
    // differential): tuples and bindings touching the entity die with it,
    // and the other endpoints' docs are marked dirty in the change log.
    let (deleted, affected) = DatastoreRepository::new(&state.db)
        .delete_entity_cascade(store.id, &entity_id)
        .await?;
    Ok(Json(EntityDeletedResponse {
        deleted,
        cascaded: affected,
    }))
}

/// Acknowledgement of an attribute replacement.
#[derive(Debug, Serialize, ToSchema)]
struct AttributesUpdatedResponse {
    entity_id: String,
    updated: bool,
}

/// Replace an entity's attribute map (typed, validated). PUT semantics keep
/// the contract obvious; PATCH-merge can layer on later without breakage.
#[utoipa::path(
    put,
    path = "/orgs/{org}/namespaces/{ns}/datastore/entities/{entity_id}/attributes",
    tag = "datastore",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("ns" = String, Path, description = "Namespace slug"),
        ("entity_id" = String, Path, description = "Entity ID")
    ),
    responses(
        (status = 200, description = "Entity attributes replaced", body = AttributesUpdatedResponse),
        (status = 400, description = "Attributes violate the model", body = ProblemDetails),
        (status = 404, description = "Entity (or datastore) not found", body = ProblemDetails)
    ),
    security(("bearer_jwt" = []))
)]
async fn put_attributes(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, ns, entity_id)): Path<(String, String, String)>,
    Json(attributes): Json<serde_json::Map<String, Value>>,
) -> ApiResult<Json<AttributesUpdatedResponse>> {
    let resolved = authorize(&state, &user, &org, &ns, true).await?;
    let store = require_store(&state, &resolved).await?;
    let repo = DatastoreRepository::new(&state.db);
    let mut entity = repo
        .get_entity(store.id, &entity_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("entity '{entity_id}' not found")))?;
    store
        .model
        .validate_attributes(&entity.entity_type, &attributes)
        .map_err(ApiError::BadRequest)?;
    entity.attributes = attributes;
    repo.upsert_entity(store.id, &entity).await?;
    Ok(Json(AttributesUpdatedResponse {
        entity_id,
        updated: true,
    }))
}

// ---------------------------------------------------------------------------
// Role bindings
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct BindingListParams {
    subject: Option<String>,
    role: Option<String>,
    limit: Option<i64>,
    cursor: Option<String>,
}

/// List role bindings, keyset-paginated (Plan 07 Phase E; closes R2-01).
#[utoipa::path(
    get,
    path = "/orgs/{org}/namespaces/{ns}/datastore/role-bindings",
    tag = "datastore",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("ns" = String, Path, description = "Namespace slug"),
        ("subject" = Option<String>, Query, description = "Filter by subject"),
        ("role" = Option<String>, Query, description = "Filter by role"),
        ("limit" = Option<i64>, Query, description = "Page size (default 50, max 200)"),
        ("cursor" = Option<String>, Query, description = "Opaque cursor from the previous page's next_cursor")
    ),
    responses(
        (status = 200, description = "One page of role bindings with a next_cursor to resume",
         body = Paginated<RoleBinding>),
        (status = 400, description = "limit out of range or cursor invalid", body = ProblemDetails),
        (status = 404, description = "No datastore provisioned for this namespace", body = ProblemDetails)
    ),
    security(("bearer_jwt" = []))
)]
async fn list_bindings(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, ns)): Path<(String, String)>,
    Query(params): Query<BindingListParams>,
) -> ApiResult<Json<Paginated<PageRow<RoleBinding>>>> {
    let resolved = authorize(&state, &user, &org, &ns, false).await?;
    let store = require_store(&state, &resolved).await?;
    let page = PageQuery {
        limit: params.limit,
        cursor: params.cursor,
    }
    .validate()?;
    let rows = DatastoreRepository::new(&state.db)
        .list_bindings_page(
            store.id,
            params.subject.as_deref(),
            params.role.as_deref(),
            page.limit + 1,
            page.after.as_ref(),
        )
        .await?;
    Ok(Json(Paginated::from_rows(rows, &page, PageRow::key)))
}

/// Acknowledgement of a role-binding grant.
#[derive(Debug, Serialize, ToSchema)]
struct BindingAddedResponse {
    bound: bool,
}

/// Grant a role to a subject (namespace-wide; the role must exist in the model).
#[utoipa::path(
    post,
    path = "/orgs/{org}/namespaces/{ns}/datastore/role-bindings",
    tag = "datastore",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("ns" = String, Path, description = "Namespace slug")
    ),
    request_body = RoleBinding,
    responses(
        (status = 200, description = "Role binding added", body = BindingAddedResponse),
        (status = 400, description = "Role not defined in the model, or scoped binding (unsupported)", body = ProblemDetails),
        (status = 404, description = "No datastore provisioned for this namespace", body = ProblemDetails)
    ),
    security(("bearer_jwt" = []))
)]
async fn add_binding(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, ns)): Path<(String, String)>,
    Json(binding): Json<RoleBinding>,
) -> ApiResult<Json<BindingAddedResponse>> {
    let resolved = authorize(&state, &user, &org, &ns, true).await?;
    let store = require_store(&state, &resolved).await?;
    if store.model.role(&binding.role).is_none() {
        return Err(ApiError::BadRequest(format!(
            "role '{}' is not defined in the model",
            binding.role
        )));
    }
    // Scoped bindings are stored-but-not-yet-materialized (D2). Accepting
    // one today would silently widen a scoped grant into a GLOBAL grant at
    // publish time — reject loudly instead (fail closed, no surprises).
    if !binding.scope.is_empty() {
        return Err(ApiError::BadRequest(
            "scoped role bindings are not supported yet — omit `scope` for a \
             namespace-wide binding (resource-scoped bindings land in D2)"
                .to_string(),
        ));
    }
    DatastoreRepository::new(&state.db)
        .add_binding(store.id, &binding)
        .await?;
    Ok(Json(BindingAddedResponse { bound: true }))
}

/// Result of a record delete (`deleted: false` = the record did not exist).
#[derive(Debug, Serialize, ToSchema)]
struct RecordDeletedResponse {
    deleted: bool,
}

/// Revoke a role binding (idempotent; `deleted: false` when absent).
#[utoipa::path(
    delete,
    path = "/orgs/{org}/namespaces/{ns}/datastore/role-bindings",
    tag = "datastore",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("ns" = String, Path, description = "Namespace slug")
    ),
    request_body = RoleBinding,
    responses(
        (status = 200, description = "Role binding removed", body = RecordDeletedResponse),
        (status = 404, description = "No datastore provisioned for this namespace", body = ProblemDetails)
    ),
    security(("bearer_jwt" = []))
)]
async fn remove_binding(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, ns)): Path<(String, String)>,
    Json(binding): Json<RoleBinding>,
) -> ApiResult<Json<RecordDeletedResponse>> {
    let resolved = authorize(&state, &user, &org, &ns, true).await?;
    let store = require_store(&state, &resolved).await?;
    let deleted = DatastoreRepository::new(&state.db)
        .delete_binding(store.id, &binding)
        .await?;
    Ok(Json(RecordDeletedResponse { deleted }))
}

// ---------------------------------------------------------------------------
// Relationship tuples
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct TupleListParams {
    object: Option<String>,
    relation: Option<String>,
    subject: Option<String>,
    limit: Option<i64>,
    cursor: Option<String>,
}

/// List relationship tuples, keyset-paginated (Plan 07 Phase E; closes R2-01).
#[utoipa::path(
    get,
    path = "/orgs/{org}/namespaces/{ns}/datastore/tuples",
    tag = "datastore",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("ns" = String, Path, description = "Namespace slug"),
        ("object" = Option<String>, Query, description = "Filter by object"),
        ("relation" = Option<String>, Query, description = "Filter by relation"),
        ("subject" = Option<String>, Query, description = "Filter by subject"),
        ("limit" = Option<i64>, Query, description = "Page size (default 50, max 200)"),
        ("cursor" = Option<String>, Query, description = "Opaque cursor from the previous page's next_cursor")
    ),
    responses(
        (status = 200, description = "One page of relationship tuples with a next_cursor to resume",
         body = Paginated<RelationTuple>),
        (status = 400, description = "limit out of range or cursor invalid", body = ProblemDetails),
        (status = 404, description = "No datastore provisioned for this namespace", body = ProblemDetails)
    ),
    security(("bearer_jwt" = []))
)]
async fn list_tuples(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, ns)): Path<(String, String)>,
    Query(params): Query<TupleListParams>,
) -> ApiResult<Json<Paginated<PageRow<RelationTuple>>>> {
    let resolved = authorize(&state, &user, &org, &ns, false).await?;
    let store = require_store(&state, &resolved).await?;
    let page = PageQuery {
        limit: params.limit,
        cursor: params.cursor,
    }
    .validate()?;
    let rows = DatastoreRepository::new(&state.db)
        .list_tuples_page(
            store.id,
            params.object.as_deref(),
            params.relation.as_deref(),
            params.subject.as_deref(),
            page.limit + 1,
            page.after.as_ref(),
        )
        .await?;
    Ok(Json(Paginated::from_rows(rows, &page, PageRow::key)))
}

/// Acknowledgement of a tuple write.
#[derive(Debug, Serialize, ToSchema)]
struct TupleWrittenResponse {
    written: bool,
}

/// Write a relationship tuple (the relation must exist in the model).
#[utoipa::path(
    post,
    path = "/orgs/{org}/namespaces/{ns}/datastore/tuples",
    tag = "datastore",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("ns" = String, Path, description = "Namespace slug")
    ),
    request_body = RelationTuple,
    responses(
        (status = 200, description = "Relationship tuple written", body = TupleWrittenResponse),
        (status = 400, description = "Relation not defined in the model", body = ProblemDetails),
        (status = 404, description = "No datastore provisioned for this namespace", body = ProblemDetails)
    ),
    security(("bearer_jwt" = []))
)]
async fn write_tuple(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, ns)): Path<(String, String)>,
    Json(tuple): Json<RelationTuple>,
) -> ApiResult<Json<TupleWrittenResponse>> {
    let resolved = authorize(&state, &user, &org, &ns, true).await?;
    let store = require_store(&state, &resolved).await?;
    if store.model.relation(&tuple.relation).is_none() {
        return Err(ApiError::BadRequest(format!(
            "relation '{}' is not defined in the model",
            tuple.relation
        )));
    }
    DatastoreRepository::new(&state.db)
        .write_tuple(store.id, &tuple)
        .await?;
    Ok(Json(TupleWrittenResponse { written: true }))
}

/// Remove a relationship tuple (idempotent; `deleted: false` when absent).
#[utoipa::path(
    delete,
    path = "/orgs/{org}/namespaces/{ns}/datastore/tuples",
    tag = "datastore",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("ns" = String, Path, description = "Namespace slug")
    ),
    request_body = RelationTuple,
    responses(
        (status = 200, description = "Relationship tuple removed", body = RecordDeletedResponse),
        (status = 404, description = "No datastore provisioned for this namespace", body = ProblemDetails)
    ),
    security(("bearer_jwt" = []))
)]
async fn remove_tuple(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, ns)): Path<(String, String)>,
    Json(tuple): Json<RelationTuple>,
) -> ApiResult<Json<RecordDeletedResponse>> {
    let resolved = authorize(&state, &user, &org, &ns, true).await?;
    let store = require_store(&state, &resolved).await?;
    let deleted = DatastoreRepository::new(&state.db)
        .delete_tuple(store.id, &tuple)
        .await?;
    Ok(Json(RecordDeletedResponse { deleted }))
}

// ---------------------------------------------------------------------------
// Publish + versions
// ---------------------------------------------------------------------------

/// The freshly published immutable data-bundle version.
#[derive(Debug, Serialize, ToSchema)]
struct PublishResponse {
    version: i64,
    /// `sha256:<hex>` over the materialized document.
    checksum: String,
    counts: RecordCounts,
    published_at: String,
}

/// Cut a new immutable, checksummed data-bundle version and wake the fleet.
#[utoipa::path(
    post,
    path = "/orgs/{org}/namespaces/{ns}/datastore/publish",
    tag = "datastore",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("ns" = String, Path, description = "Namespace slug")
    ),
    responses(
        (status = 200, description = "New immutable data-bundle version published",
         body = PublishResponse),
        (status = 404, description = "No datastore provisioned for this namespace", body = ProblemDetails)
    ),
    security(("bearer_jwt" = []))
)]
async fn publish(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, ns)): Path<(String, String)>,
) -> ApiResult<Json<PublishResponse>> {
    let resolved = authorize(&state, &user, &org, &ns, true).await?;
    let store = require_store(&state, &resolved).await?;
    let published = DatastoreRepository::new(&state.db)
        .publish(&store, &user.id.to_string())
        .await?;
    notify_published(&state, &resolved, store.id, &published).await;

    Ok(Json(PublishResponse {
        version: published.version,
        checksum: published.checksum.clone(),
        counts: RecordCounts {
            entities: published.entity_count,
            role_bindings: published.binding_count,
            tuples: published.tuple_count,
        },
        published_at: published.published_at.clone(),
    }))
}

// ---------------------------------------------------------------------------
// Model migrations (Plan 12): dry-run plan + impact analysis
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, utoipa::ToSchema)]
struct PlanMigrationRequest {
    transforms: Vec<migration::ModelTransform>,
}

/// Which materialized documents a migration touches (sampled).
#[derive(Debug, Serialize, ToSchema)]
struct DocsChangedSummary {
    /// Total documents that would be re-materialized (upserts + tombstones).
    total: usize,
    /// Up to 100 affected entity ids.
    sample: Vec<String>,
}

/// The pre-migration side of a plan report.
#[derive(Debug, Serialize, ToSchema)]
struct PlanStateBefore {
    /// `sha256:<hex>` over the materialized document.
    checksum: String,
    /// Materialized entity-document count.
    entities: usize,
    /// The model version the plan was computed against — `apply` is guarded
    /// on it (R2-04).
    model_version: i64,
}

/// The post-migration side of a plan report.
#[derive(Debug, Serialize, ToSchema)]
struct PlanStateAfter {
    /// `sha256:<hex>` over the materialized document.
    checksum: String,
    /// Materialized entity-document count.
    entities: usize,
    /// The full model definition the migration produces.
    model: ModelDefinition,
}

/// Dry-run migration plan: record-level ops, blockers, and access impact.
#[derive(Debug, Serialize, ToSchema)]
struct MigrationPlanResponse {
    /// False when blockers exist — `apply` will refuse with 409.
    applyable: bool,
    record_ops: Vec<migration::RecordOp>,
    blockers: Vec<migration::PlanBlocker>,
    /// Access-impact diff; `null` when the plan is blocked (a plan that
    /// cannot apply has no meaningful "after" state).
    impact: Option<impact::ImpactReport>,
    docs_changed: DocsChangedSummary,
    before: PlanStateBefore,
    after: PlanStateAfter,
}

/// Dry-run a model migration: NOTHING is mutated. Returns the record-level
/// plan (exact affected-row counts per transform), any blockers (fail-closed
/// coercion errors, non-empty relation removals), and — when applyable — the
/// access-impact report: which principals gain or lose access, computed by
/// materializing the proposed state and diffing engine-visible access
/// against current. A pure rename must report decision_neutral: true.
#[utoipa::path(
    post,
    path = "/orgs/{org}/namespaces/{ns}/datastore/migrations/plan",
    tag = "datastore",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("ns" = String, Path, description = "Namespace slug")
    ),
    request_body = PlanMigrationRequest,
    responses(
        (status = 200, description = "Dry-run migration plan + impact report (no mutation)",
         body = MigrationPlanResponse),
        (status = 400, description = "Invalid transform (unknown source, name collision, bad default)",
         body = ProblemDetails),
        (status = 404, description = "No datastore provisioned for this namespace", body = ProblemDetails)
    ),
    security(("bearer_jwt" = []))
)]
async fn plan_migration(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, ns)): Path<(String, String)>,
    Json(req): Json<PlanMigrationRequest>,
) -> ApiResult<Json<MigrationPlanResponse>> {
    let resolved = authorize(&state, &user, &org, &ns, true).await?;
    let store = require_store(&state, &resolved).await?;
    let prepared = prepare_migration(&state, &store, &req.transforms).await?;
    Ok(Json(prepared.report(&store)))
}

/// Map the repository's model-version guard failure (R2-04) to 409 Conflict:
/// the migration was planned against a model that a concurrent writer has
/// since changed — never silently rewrite entities from a stale snapshot.
/// (The generic `DatabaseError → ApiError` mapping sends `VersionConflict` to
/// 412, which is right for `If-Match` flows but wrong here: no precondition
/// header was involved.)
fn model_conflict_to_api(e: DatabaseError) -> ApiError {
    match e {
        DatabaseError::VersionConflict(msg) => ApiError::Conflict(msg),
        e => e.into(),
    }
}

/// Apply a planned migration ATOMICALLY, then publish the new data version
/// so the fleet converges via the existing delta path. The plan is
/// recomputed server-side from the transforms — a client can never smuggle
/// a stale or hand-edited plan past the blockers. Blocked plans are refused
/// with 409 and the blocker list; nothing is mutated. The model UPDATE is
/// guarded on the plan's `model_before` version (R2-04): a concurrent
/// migration makes this one fail with 409 instead of clobbering. Because a
/// successful apply publishes and fans out to the fleet, the endpoint accepts
/// an `Idempotency-Key` (R2-05): a retried timeout replays the original
/// response instead of double-applying and double-propagating.
#[utoipa::path(
    post,
    path = "/orgs/{org}/namespaces/{ns}/datastore/migrations/apply",
    tag = "datastore",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("ns" = String, Path, description = "Namespace slug"),
        ("Idempotency-Key" = Option<String>, Header,
         description = "Optional retry-safety key: a replay within the retention \
                        window returns the original response without re-applying \
                        the migration or re-propagating to the fleet (Plan 07 Phase D)")
    ),
    request_body = PlanMigrationRequest,
    responses(
        (status = 200, description = "Migration applied atomically + new data version published",
         body = ApplyMigrationResponse),
        (status = 400, description = "Invalid transform", body = ProblemDetails),
        (status = 409, description = "Plan is blocked (coercion errors / non-empty relation), \
            the model changed concurrently since the plan's model_before (re-plan and retry), \
            or the original request for this Idempotency-Key is still in flight",
         body = ProblemDetails),
        (status = 422, description = "Idempotency-Key was already used for a different request",
         body = ProblemDetails),
        (status = 404, description = "No datastore provisioned for this namespace", body = ProblemDetails)
    ),
    security(("bearer_jwt" = []))
)]
async fn apply_migration(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, ns)): Path<(String, String)>,
    headers: HeaderMap,
    Json(req): Json<PlanMigrationRequest>,
) -> ApiResult<Response> {
    // Propagation-triggering POST (publish + fleet fan-out): a retried
    // request must not apply the migration twice (Plan 07 Phase D, R2-05).
    let transforms_json = serde_json::to_string(&req.transforms)
        .map_err(|e| ApiError::Internal(format!("serialize transforms: {e}")))?;
    let fingerprint = idempotency::fingerprint(&["datastore.migrate", &org, &ns, &transforms_json]);
    let scope_id = format!("{org}/{ns}");
    let db = state.db.clone();
    idempotency::run(
        &db,
        &headers,
        "datastore.migrate",
        &scope_id,
        &fingerprint,
        || apply_migration_inner(state, user, org, ns, req),
    )
    .await
}

/// A published version pointer (version + checksum).
#[derive(Debug, Serialize, ToSchema)]
struct PublishedRef {
    version: i64,
    /// `sha256:<hex>` over the materialized document.
    checksum: String,
}

/// Result of an atomically applied migration + the publish it triggered.
#[derive(Debug, Serialize, ToSchema)]
struct ApplyMigrationResponse {
    /// The new model-shape version.
    model_version: i64,
    record_ops: Vec<migration::RecordOp>,
    /// Access-impact diff computed by the server-side re-plan.
    impact: Option<impact::ImpactReport>,
    /// Materialized documents re-published (upserts + tombstones).
    docs_changed: usize,
    published: PublishedRef,
}

/// The actual migration side effect; runs at most once per idempotency key.
async fn apply_migration_inner(
    state: Arc<AppState>,
    user: AuthenticatedUser,
    org: String,
    ns: String,
    req: PlanMigrationRequest,
) -> ApiResult<(StatusCode, Value)> {
    let resolved = authorize(&state, &user, &org, &ns, true).await?;
    let store = require_store(&state, &resolved).await?;
    let prepared = prepare_migration(&state, &store, &req.transforms).await?;

    if !prepared.plan.applyable() {
        return Err(ApiError::Conflict(
            serde_json::to_string(&json!({
                "error": "migration plan is blocked (fail closed) — resolve the blockers or \
                          adjust the transforms",
                "blockers": prepared.plan.blockers,
            }))
            .unwrap_or_else(|_| "migration plan is blocked".to_string()),
        ));
    }

    let repo = DatastoreRepository::new(&state.db);
    // One transaction: records + model + model_version + history + outbox,
    // guarded on the model_version the plan was computed against (R2-04).
    let model_version = repo
        .apply_migration(
            &store,
            &prepared.plan,
            &prepared.dirty,
            &user.id.to_string(),
        )
        .await
        .map_err(model_conflict_to_api)?;

    // Publish the post-migration data version so agents converge (snapshot
    // lineage pinned at the migration's change_seq). A crash between the
    // committed apply and this publish is recoverable by re-publishing —
    // the records, model, and outbox are already consistent.
    let updated = require_store(&state, &resolved).await?;
    let published = repo.publish(&updated, &user.id.to_string()).await?;
    notify_published(&state, &resolved, updated.id, &published).await;

    // Typed for the contract (R2-06); serialized to a Value at the
    // idempotency boundary, which persists the body for replay.
    let response = ApplyMigrationResponse {
        model_version,
        record_ops: prepared.plan.record_ops,
        impact: prepared.impact,
        docs_changed: prepared.dirty.len(),
        published: PublishedRef {
            version: published.version,
            checksum: published.checksum,
        },
    };
    let body = serde_json::to_value(&response)
        .map_err(|e| ApiError::Internal(format!("serialize migration response: {e}")))?;
    Ok((StatusCode::OK, body))
}

/// The append-only migration history for a datastore.
#[derive(Debug, Serialize, ToSchema)]
struct MigrationHistoryResponse {
    /// Current model-shape version (0 = never migrated).
    model_version: i64,
    /// Newest first: `{model_version, transforms, author, model_before_hash,
    /// model_after_hash, created_at}` per applied migration.
    #[schema(value_type = Vec<Object>)]
    migrations: Vec<Value>,
}

/// The append-only migration history (newest first): transforms, author,
/// before/after model hashes per model version.
#[utoipa::path(
    get,
    path = "/orgs/{org}/namespaces/{ns}/datastore/migrations",
    tag = "datastore",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("ns" = String, Path, description = "Namespace slug")
    ),
    responses(
        (status = 200, description = "Migration history", body = MigrationHistoryResponse),
        (status = 404, description = "No datastore provisioned for this namespace", body = ProblemDetails)
    ),
    security(("bearer_jwt" = []))
)]
async fn list_migrations(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, ns)): Path<(String, String)>,
) -> ApiResult<Json<MigrationHistoryResponse>> {
    let resolved = authorize(&state, &user, &org, &ns, false).await?;
    let store = require_store(&state, &resolved).await?;
    let history = DatastoreRepository::new(&state.db)
        .list_model_versions(store.id)
        .await?;
    Ok(Json(MigrationHistoryResponse {
        model_version: store.model_version,
        migrations: history,
    }))
}

/// Result of rolling back a migration (a NEW forward model version).
#[derive(Debug, Serialize, ToSchema)]
struct RollbackMigrationResponse {
    /// The migration (model version) that was rolled back.
    rolled_back: i64,
    /// The new model-shape version the inverse migration produced.
    model_version: i64,
    /// The composed inverse transforms that were applied.
    transforms: Vec<migration::ModelTransform>,
    /// Access-impact diff of the inverse migration.
    impact: Option<impact::ImpactReport>,
    published: PublishedRef,
}

/// Roll back an applied migration by composing its INVERSE transforms into
/// a NEW forward migration (ADR-3: append-only history — audit sees the
/// change and its undo as two events, never a rewritten past). The inverse
/// chain runs through the same plan+apply pipeline, so it is impact-checked
/// and fails closed like any other migration. Irreversible transforms
/// (remove_attribute) are refused: restore record data from the immutable
/// pre-migration data version instead.
#[utoipa::path(
    post,
    path = "/orgs/{org}/namespaces/{ns}/datastore/migrations/{version}/rollback",
    tag = "datastore",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("ns" = String, Path, description = "Namespace slug"),
        ("version" = i64, Path, description = "Model version (migration) to roll back")
    ),
    responses(
        (status = 200, description = "Inverse migration applied as a new forward model version",
         body = RollbackMigrationResponse),
        (status = 404, description = "No such migration", body = ProblemDetails),
        (status = 409, description = "Migration is irreversible, the inverse plan is blocked, \
            or the model changed concurrently (re-plan and retry)",
         body = ProblemDetails)
    ),
    security(("bearer_jwt" = []))
)]
async fn rollback_migration(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, ns, version)): Path<(String, String, i64)>,
) -> ApiResult<Json<RollbackMigrationResponse>> {
    let resolved = authorize(&state, &user, &org, &ns, true).await?;
    let store = require_store(&state, &resolved).await?;
    let repo = DatastoreRepository::new(&state.db);

    let (transforms, model_before) = repo
        .get_model_version(store.id, version)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("model version {version} not found")))?;
    let model_before = model_before.ok_or_else(|| {
        ApiError::Conflict(format!(
            "model version {version} predates stored model snapshots and cannot be \
             auto-rolled-back"
        ))
    })?;

    let inverse =
        migration::compose_rollback(&transforms, &model_before).map_err(ApiError::Conflict)?;

    // Same pipeline as a hand-written migration: plan against the CURRENT
    // store (intermediate migrations may make the inverse invalid — that
    // surfaces here as a 400/blocker, fail closed), then atomic apply +
    // publish.
    let prepared = prepare_migration(&state, &store, &inverse).await?;
    if !prepared.plan.applyable() {
        return Err(ApiError::Conflict(
            serde_json::to_string(&json!({
                "error": "inverse migration plan is blocked (fail closed)",
                "blockers": prepared.plan.blockers,
            }))
            .unwrap_or_else(|_| "inverse migration plan is blocked".to_string()),
        ));
    }
    let model_version = repo
        .apply_migration(
            &store,
            &prepared.plan,
            &prepared.dirty,
            &user.id.to_string(),
        )
        .await
        .map_err(model_conflict_to_api)?;
    let updated = require_store(&state, &resolved).await?;
    let published = repo.publish(&updated, &user.id.to_string()).await?;
    notify_published(&state, &resolved, updated.id, &published).await;

    Ok(Json(RollbackMigrationResponse {
        rolled_back: version,
        model_version,
        transforms: inverse,
        impact: prepared.impact,
        published: PublishedRef {
            version: published.version,
            checksum: published.checksum,
        },
    }))
}

/// Everything a dry-run computes; `apply` persists exactly this state.
struct PreparedMigration {
    plan: migration::MigrationPlan,
    doc_before: Value,
    doc_after: Value,
    /// FULL dirty set for the outbox: (entity_id, tombstone).
    dirty: Vec<(String, bool)>,
    impact: Option<impact::ImpactReport>,
}

async fn prepare_migration(
    state: &AppState,
    store: &DatastoreRecord,
    transforms: &[migration::ModelTransform],
) -> ApiResult<PreparedMigration> {
    let repo = DatastoreRepository::new(&state.db);

    // Full record sets — the planner and both materializations need them.
    let entities = repo.list_entities(store.id, None).await?;
    let bindings = repo.list_bindings(store.id, None, None).await?;
    let tuples = repo.list_tuples(store.id, None, None, None).await?;

    let plan = migration::plan(transforms, &store.model, &entities, &bindings, &tuples)
        .map_err(ApiError::BadRequest)?;

    // Materialize both worlds. Deterministic ordering (BTreeMap/BTreeSet in
    // materialize) keeps the checksums meaningful.
    let doc_before = materialize(&store.model, &entities, &bindings, &tuples);
    let doc_after = materialize(
        &plan.model_after,
        &plan.entities_after,
        &plan.bindings_after,
        &plan.tuples_after,
    );

    // Structural diff → the exact outbox dirty set: docs that changed or
    // appeared get an upsert mark; docs that vanished get a tombstone.
    let by_id = |doc: &Value| -> std::collections::BTreeMap<String, Value> {
        doc["entities"]
            .as_array()
            .map(|ents| {
                ents.iter()
                    .filter_map(|e| e["id"].as_str().map(|id| (id.to_string(), e.clone())))
                    .collect()
            })
            .unwrap_or_default()
    };
    let before_map = by_id(&doc_before);
    let after_map = by_id(&doc_after);
    let mut dirty: Vec<(String, bool)> = Vec::new();
    for (id, doc) in &before_map {
        match after_map.get(id) {
            None => dirty.push((id.clone(), true)),
            Some(after_doc) if after_doc != doc => dirty.push((id.clone(), false)),
            _ => {}
        }
    }
    for id in after_map.keys() {
        if !before_map.contains_key(id) {
            dirty.push((id.clone(), false));
        }
    }
    dirty.sort();

    // Access impact via the real engine — skipped when blocked, because a
    // plan that cannot apply has no meaningful "after" state.
    let impact = if plan.applyable() {
        let specs = |m: &ModelDefinition| -> Vec<impact::RelationSpec> {
            m.relations
                .iter()
                .map(|r| impact::RelationSpec {
                    name: r.name.clone(),
                    traversal: r.traversal,
                })
                .collect()
        };
        let profile_before = impact::access_profile(&doc_before, &specs(&store.model))
            .map_err(ApiError::Internal)?;
        let profile_after = impact::access_profile(&doc_after, &specs(&plan.model_after))
            .map_err(ApiError::Internal)?;
        let maps = migration::rename_maps(transforms);
        Some(impact::diff(
            &impact::normalize(&profile_before, &maps),
            &profile_after,
        ))
    } else {
        None
    };

    Ok(PreparedMigration {
        plan,
        doc_before,
        doc_after,
        dirty,
        impact,
    })
}

impl PreparedMigration {
    fn report(&self, store: &DatastoreRecord) -> MigrationPlanResponse {
        let checksum = |doc: &Value| -> String {
            use sha2::{Digest, Sha256};
            format!(
                "sha256:{}",
                hex::encode(Sha256::digest(doc.to_string().as_bytes()))
            )
        };
        let count = |doc: &Value| doc["entities"].as_array().map(|a| a.len()).unwrap_or(0);
        let sample: Vec<String> = self
            .dirty
            .iter()
            .take(100)
            .map(|(id, _)| id.clone())
            .collect();
        MigrationPlanResponse {
            applyable: self.plan.applyable(),
            record_ops: self.plan.record_ops.clone(),
            blockers: self.plan.blockers.clone(),
            impact: self.impact.clone(),
            docs_changed: DocsChangedSummary {
                total: self.dirty.len(),
                sample,
            },
            before: PlanStateBefore {
                checksum: checksum(&self.doc_before),
                entities: count(&self.doc_before),
                model_version: store.model_version,
            },
            after: PlanStateAfter {
                checksum: checksum(&self.doc_after),
                entities: count(&self.doc_after),
                model: self.plan.model_after.clone(),
            },
        }
    }
}

#[derive(Debug, Deserialize)]
struct ChangesParams {
    /// Last sequence the replica has applied (exclusive).
    #[serde(default)]
    since: i64,
    /// Max deltas per page (post-dedup entities, not raw log rows).
    limit: Option<i64>,
}

/// One delta: an upserted materialized document or a tombstone.
#[derive(Debug, Serialize, ToSchema)]
struct ChangeDelta {
    /// `upsert` or `delete`.
    op: String,
    entity_id: String,
    /// The freshly materialized entity document (present on `upsert`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Object)]
    document: Option<Value>,
}

impl ChangeDelta {
    fn delete(entity_id: String) -> Self {
        Self {
            op: "delete".to_string(),
            entity_id,
            document: None,
        }
    }
}

/// The durable delta pull. Either a page of deltas (`snapshot_required:
/// false`) or a directive to fall back to a full snapshot deploy because
/// `since` predates the compaction floor.
#[derive(Debug, Serialize, ToSchema)]
struct ChangesResponse {
    /// True when deltas can no longer bridge the gap — reload from the
    /// current published version instead.
    snapshot_required: bool,
    /// Head of the change stream at read time.
    head_seq: i64,
    /// Current published version to snapshot from (present when
    /// `snapshot_required`).
    #[serde(skip_serializing_if = "Option::is_none")]
    current_version: Option<i64>,
    /// Echo of the requested resume point (present on a delta page).
    #[serde(skip_serializing_if = "Option::is_none")]
    since: Option<i64>,
    /// Deduped, latest-state deltas (present on a delta page).
    #[serde(skip_serializing_if = "Option::is_none")]
    deltas: Option<Vec<ChangeDelta>>,
}

/// GET …/datastore/changes?since=N — the durable delta pull. Replicas ask
/// "everything after my seq"; a lost notification can never lose data
/// because this log is the source, not the event. Entities are DEDUPED to
/// their latest state (a record churned 50 times syncs once) and each is
/// materialized fresh via three indexed point queries. When `since` is
/// older than the compaction floor the response says snapshot_required —
/// the replica falls back to a full verified deploy.
#[utoipa::path(
    get,
    path = "/orgs/{org}/namespaces/{ns}/datastore/changes",
    tag = "datastore",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("ns" = String, Path, description = "Namespace slug"),
        ("since" = Option<i64>, Query, description = "Last applied sequence (exclusive; default 0)"),
        ("limit" = Option<i64>, Query, description = "Max deltas per page (default 500, max 2000)")
    ),
    responses(
        (status = 200, description = "Durable delta pull since a sequence", body = ChangesResponse),
        (status = 404, description = "No datastore provisioned for this namespace", body = ProblemDetails)
    ),
    security(("bearer_jwt" = []))
)]
async fn get_changes(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, ns)): Path<(String, String)>,
    Query(params): Query<ChangesParams>,
) -> ApiResult<Json<ChangesResponse>> {
    let resolved = authorize(&state, &user, &org, &ns, false).await?;
    let store = require_store(&state, &resolved).await?;
    let repo = DatastoreRepository::new(&state.db);

    let limit = params.limit.unwrap_or(500).clamp(1, 2000);
    let (head_seq, min_available, marks) =
        repo.changes_since(store.id, params.since, limit).await?;

    // Replica older than the compaction floor: deltas can no longer bridge
    // the gap — self-heal via snapshot (min_available == 0 means an empty
    // log, which is only a gap if the head has moved past `since`). This
    // must apply to since=0 followers too: time-based retention can prune
    // the early log before a first publish, and telling such a follower
    // "you're current" with an empty delta list would be a silent gap.
    let compacted_away = params.since < min_available.saturating_sub(1)
        || (min_available == 0 && head_seq > params.since && marks.is_empty());
    if compacted_away {
        return Ok(Json(ChangesResponse {
            snapshot_required: true,
            head_seq,
            current_version: Some(store.current_version),
            since: None,
            deltas: None,
        }));
    }

    let mut deltas = Vec::with_capacity(marks.len());
    for (entity_id, tombstone) in marks {
        if tombstone {
            deltas.push(ChangeDelta::delete(entity_id));
            continue;
        }
        let (entity, bindings, tuples) = repo.entity_view(store.id, &entity_id).await?;
        match crate::domain::datastore::materialize_one(
            &store.model,
            &entity_id,
            entity.as_ref(),
            &bindings,
            &tuples,
        ) {
            Some(document) => deltas.push(ChangeDelta {
                op: "upsert".to_string(),
                entity_id,
                document: Some(document),
            }),
            // Nothing materializes anymore (e.g. its last tuple went away
            // and it never had a record): tombstone converges the replica.
            None => deltas.push(ChangeDelta::delete(entity_id)),
        }
    }

    Ok(Json(ChangesResponse {
        snapshot_required: false,
        head_seq,
        current_version: None,
        since: Some(params.since),
        deltas: Some(deltas),
    }))
}

/// All published data-bundle versions, newest first.
#[derive(Debug, Serialize, ToSchema)]
struct VersionListResponse {
    versions: Vec<PublishedVersion>,
}

/// List all published data-bundle versions, newest first.
#[utoipa::path(
    get,
    path = "/orgs/{org}/namespaces/{ns}/datastore/versions",
    tag = "datastore",
    operation_id = "datastore_list_versions",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("ns" = String, Path, description = "Namespace slug")
    ),
    responses(
        (status = 200, description = "List of published versions", body = VersionListResponse),
        (status = 404, description = "No datastore provisioned for this namespace", body = ProblemDetails)
    ),
    security(("bearer_jwt" = []))
)]
async fn list_versions(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, ns)): Path<(String, String)>,
) -> ApiResult<Json<VersionListResponse>> {
    let resolved = authorize(&state, &user, &org, &ns, false).await?;
    let store = require_store(&state, &resolved).await?;
    let versions = DatastoreRepository::new(&state.db)
        .list_versions(store.id)
        .await?;
    Ok(Json(VersionListResponse { versions }))
}

/// One published version plus its materialized document.
#[derive(Debug, Serialize, ToSchema)]
struct VersionDocumentResponse {
    version: i64,
    /// `sha256:<hex>` over the materialized document.
    checksum: String,
    /// Change-stream position at publish time (delta resume point).
    change_seq: i64,
    /// Model-shape version the document was materialized under.
    model_version: i64,
    published_at: String,
    /// The exact `{"entities": [...]}` payload agents load.
    #[schema(value_type = Object)]
    document: Value,
}

/// Returns the materialized document — the exact payload an agent POSTs to
/// its own /api/v1/data endpoint (or reaper-sync applies).
#[utoipa::path(
    get,
    path = "/orgs/{org}/namespaces/{ns}/datastore/versions/{version}",
    tag = "datastore",
    operation_id = "datastore_get_version",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("ns" = String, Path, description = "Namespace slug"),
        ("version" = i64, Path, description = "Version number")
    ),
    responses(
        (status = 200, description = "Materialized version document", body = VersionDocumentResponse),
        (status = 404, description = "Version (or datastore) not found", body = ProblemDetails)
    ),
    security(("bearer_jwt" = []))
)]
async fn get_version(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, ns, version)): Path<(String, String, i64)>,
) -> ApiResult<Json<VersionDocumentResponse>> {
    let resolved = authorize(&state, &user, &org, &ns, false).await?;
    let store = require_store(&state, &resolved).await?;
    let (meta, document) = DatastoreRepository::new(&state.db)
        .get_version_document(store.id, version)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("version {version} not found")))?;
    let document: Value = serde_json::from_str(&document)
        .map_err(|e| ApiError::Internal(format!("corrupt stored document: {e}")))?;
    Ok(Json(VersionDocumentResponse {
        version: meta.version,
        checksum: meta.checksum,
        change_seq: meta.change_seq,
        model_version: meta.model_version,
        published_at: meta.published_at,
        document,
    }))
}
