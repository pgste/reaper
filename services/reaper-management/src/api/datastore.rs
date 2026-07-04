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
    response::Json,
    routing::{get, post, put},
    Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    api::error::{ApiError, ApiResult},
    api::orgs::resolve_org,
    auth::{middleware::AuthenticatedUser, middleware::RequireAuth, scopes::Scope},
    db::repositories::datastore::DatastoreRecord,
    db::repositories::{DatastoreRepository, NamespaceRepository, OrganizationRepository},
    domain::datastore::{
        AdmEntity, DatastoreTemplate, ModelDefinition, RelationTuple, RoleBinding,
    },
    state::{AppState, ServerEvent},
};

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/orgs/{org}/namespaces/{ns}/datastore",
            get(get_datastore).post(provision),
        )
        .route(
            "/orgs/{org}/namespaces/{ns}/datastore/model",
            get(get_model).put(put_model),
        )
        .route(
            "/orgs/{org}/namespaces/{ns}/datastore/entities",
            get(list_entities).post(upsert_entity),
        )
        .route(
            "/orgs/{org}/namespaces/{ns}/datastore/entities/{entity_id}",
            get(get_entity).delete(delete_entity),
        )
        .route(
            "/orgs/{org}/namespaces/{ns}/datastore/entities/{entity_id}/attributes",
            put(put_attributes),
        )
        .route(
            "/orgs/{org}/namespaces/{ns}/datastore/role-bindings",
            get(list_bindings).post(add_binding).delete(remove_binding),
        )
        .route(
            "/orgs/{org}/namespaces/{ns}/datastore/tuples",
            get(list_tuples).post(write_tuple).delete(remove_tuple),
        )
        .route(
            "/orgs/{org}/namespaces/{ns}/datastore/publish",
            post(publish),
        )
        .route(
            "/orgs/{org}/namespaces/{ns}/datastore/versions",
            get(list_versions),
        )
        .route(
            "/orgs/{org}/namespaces/{ns}/datastore/versions/{version}",
            get(get_version),
        )
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
    let allowed = if write {
        user.has_permission(Scope::OrgAdmin)
            || user.has_permission(Scope::AgentWrite)
            || user.has_permission(Scope::Admin)
    } else {
        user.has_permission(Scope::AgentRead)
            || user.has_permission(Scope::OrgAdmin)
            || user.has_permission(Scope::Admin)
    };
    if !allowed {
        return Err(ApiError::Forbidden(if write {
            "Missing org:admin or agent:write scope".to_string()
        } else {
            "Missing agent:read or org:admin scope".to_string()
        }));
    }

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, org_ref).await?;
    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot access datastores for other organizations".to_string(),
        ));
    }

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

#[derive(Debug, Deserialize)]
struct ProvisionRequest {
    template: DatastoreTemplate,
}

async fn provision(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, ns)): Path<(String, String)>,
    Json(req): Json<ProvisionRequest>,
) -> ApiResult<Json<Value>> {
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
    Ok(Json(json!({
        "id": record.id,
        "template": record.template,
        "model": record.model,
        "current_version": record.current_version,
    })))
}

async fn get_datastore(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, ns)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    let resolved = authorize(&state, &user, &org, &ns, false).await?;
    let store = require_store(&state, &resolved).await?;
    let (entities, bindings, tuples) = DatastoreRepository::new(&state.db).counts(store.id).await?;
    Ok(Json(json!({
        "id": store.id,
        "template": store.template,
        "current_version": store.current_version,
        "counts": {
            "entities": entities,
            "role_bindings": bindings,
            "tuples": tuples,
        },
        "created_at": store.created_at,
        "updated_at": store.updated_at,
    })))
}

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

async fn get_model(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, ns)): Path<(String, String)>,
) -> ApiResult<Json<ModelDefinition>> {
    let resolved = authorize(&state, &user, &org, &ns, false).await?;
    let store = require_store(&state, &resolved).await?;
    Ok(Json(store.model))
}

async fn put_model(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, ns)): Path<(String, String)>,
    Json(model): Json<ModelDefinition>,
) -> ApiResult<Json<Value>> {
    let resolved = authorize(&state, &user, &org, &ns, true).await?;
    let store = require_store(&state, &resolved).await?;
    DatastoreRepository::new(&state.db)
        .update_model(store.id, &model)
        .await?;
    Ok(Json(json!({"updated": true})))
}

// ---------------------------------------------------------------------------
// Entities
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct EntityListParams {
    #[serde(rename = "type")]
    entity_type: Option<String>,
}

async fn list_entities(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, ns)): Path<(String, String)>,
    Query(params): Query<EntityListParams>,
) -> ApiResult<Json<Value>> {
    let resolved = authorize(&state, &user, &org, &ns, false).await?;
    let store = require_store(&state, &resolved).await?;
    let entities = DatastoreRepository::new(&state.db)
        .list_entities(store.id, params.entity_type.as_deref())
        .await?;
    Ok(Json(json!({"entities": entities, "count": entities.len()})))
}

async fn upsert_entity(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, ns)): Path<(String, String)>,
    Json(entity): Json<AdmEntity>,
) -> ApiResult<Json<Value>> {
    let resolved = authorize(&state, &user, &org, &ns, true).await?;
    let store = require_store(&state, &resolved).await?;
    store
        .model
        .validate_attributes(&entity.entity_type, &entity.attributes)
        .map_err(ApiError::BadRequest)?;
    DatastoreRepository::new(&state.db)
        .upsert_entity(store.id, &entity)
        .await?;
    Ok(Json(
        json!({"entity_id": entity.entity_id, "upserted": true}),
    ))
}

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

async fn delete_entity(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, ns, entity_id)): Path<(String, String, String)>,
) -> ApiResult<Json<Value>> {
    let resolved = authorize(&state, &user, &org, &ns, true).await?;
    let store = require_store(&state, &resolved).await?;
    let deleted = DatastoreRepository::new(&state.db)
        .delete_entity(store.id, &entity_id)
        .await?;
    Ok(Json(json!({"deleted": deleted})))
}

/// Replace an entity's attribute map (typed, validated). PUT semantics keep
/// the contract obvious; PATCH-merge can layer on later without breakage.
async fn put_attributes(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, ns, entity_id)): Path<(String, String, String)>,
    Json(attributes): Json<serde_json::Map<String, Value>>,
) -> ApiResult<Json<Value>> {
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
    Ok(Json(json!({"entity_id": entity_id, "updated": true})))
}

// ---------------------------------------------------------------------------
// Role bindings
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct BindingListParams {
    subject: Option<String>,
    role: Option<String>,
}

async fn list_bindings(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, ns)): Path<(String, String)>,
    Query(params): Query<BindingListParams>,
) -> ApiResult<Json<Value>> {
    let resolved = authorize(&state, &user, &org, &ns, false).await?;
    let store = require_store(&state, &resolved).await?;
    let bindings = DatastoreRepository::new(&state.db)
        .list_bindings(store.id, params.subject.as_deref(), params.role.as_deref())
        .await?;
    Ok(Json(
        json!({"role_bindings": bindings, "count": bindings.len()}),
    ))
}

async fn add_binding(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, ns)): Path<(String, String)>,
    Json(binding): Json<RoleBinding>,
) -> ApiResult<Json<Value>> {
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
    Ok(Json(json!({"bound": true})))
}

async fn remove_binding(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, ns)): Path<(String, String)>,
    Json(binding): Json<RoleBinding>,
) -> ApiResult<Json<Value>> {
    let resolved = authorize(&state, &user, &org, &ns, true).await?;
    let store = require_store(&state, &resolved).await?;
    let deleted = DatastoreRepository::new(&state.db)
        .delete_binding(store.id, &binding)
        .await?;
    Ok(Json(json!({"deleted": deleted})))
}

// ---------------------------------------------------------------------------
// Relationship tuples
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct TupleListParams {
    object: Option<String>,
    relation: Option<String>,
    subject: Option<String>,
}

async fn list_tuples(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, ns)): Path<(String, String)>,
    Query(params): Query<TupleListParams>,
) -> ApiResult<Json<Value>> {
    let resolved = authorize(&state, &user, &org, &ns, false).await?;
    let store = require_store(&state, &resolved).await?;
    let tuples = DatastoreRepository::new(&state.db)
        .list_tuples(
            store.id,
            params.object.as_deref(),
            params.relation.as_deref(),
            params.subject.as_deref(),
        )
        .await?;
    Ok(Json(json!({"tuples": tuples, "count": tuples.len()})))
}

async fn write_tuple(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, ns)): Path<(String, String)>,
    Json(tuple): Json<RelationTuple>,
) -> ApiResult<Json<Value>> {
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
    Ok(Json(json!({"written": true})))
}

async fn remove_tuple(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, ns)): Path<(String, String)>,
    Json(tuple): Json<RelationTuple>,
) -> ApiResult<Json<Value>> {
    let resolved = authorize(&state, &user, &org, &ns, true).await?;
    let store = require_store(&state, &resolved).await?;
    let deleted = DatastoreRepository::new(&state.db)
        .delete_tuple(store.id, &tuple)
        .await?;
    Ok(Json(json!({"deleted": deleted})))
}

// ---------------------------------------------------------------------------
// Publish + versions
// ---------------------------------------------------------------------------

async fn publish(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, ns)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    let resolved = authorize(&state, &user, &org, &ns, true).await?;
    let store = require_store(&state, &resolved).await?;
    let published = DatastoreRepository::new(&state.db)
        .publish(&store, &user.id.to_string())
        .await?;

    // Wake the fleet: agents/sync subscribed to the org event stream fetch
    // the new version and hot-swap their DataStore.
    let _ = state.event_tx.send(ServerEvent::DatastorePublished {
        datastore_id: store.id,
        org_id: resolved.org_id,
        namespace_id: Some(resolved.namespace_id),
        version: published.version,
        checksum: published.checksum.clone(),
    });

    Ok(Json(json!({
        "version": published.version,
        "checksum": published.checksum,
        "counts": {
            "entities": published.entity_count,
            "tuples": published.tuple_count,
            "role_bindings": published.binding_count,
        },
        "published_at": published.published_at,
    })))
}

async fn list_versions(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, ns)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    let resolved = authorize(&state, &user, &org, &ns, false).await?;
    let store = require_store(&state, &resolved).await?;
    let versions = DatastoreRepository::new(&state.db)
        .list_versions(store.id)
        .await?;
    Ok(Json(json!({"versions": versions})))
}

/// Returns the materialized document — the exact payload an agent POSTs to
/// its own /api/v1/data endpoint (or reaper-sync applies).
async fn get_version(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, ns, version)): Path<(String, String, i64)>,
) -> ApiResult<Json<Value>> {
    let resolved = authorize(&state, &user, &org, &ns, false).await?;
    let store = require_store(&state, &resolved).await?;
    let (meta, document) = DatastoreRepository::new(&state.db)
        .get_version_document(store.id, version)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("version {version} not found")))?;
    let document: Value = serde_json::from_str(&document)
        .map_err(|e| ApiError::Internal(format!("corrupt stored document: {e}")))?;
    Ok(Json(json!({
        "version": meta.version,
        "checksum": meta.checksum,
        "published_at": meta.published_at,
        "document": document,
    })))
}
