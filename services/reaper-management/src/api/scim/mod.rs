//! SCIM 2.0 provisioning endpoints (Plan 03, Phase 2).
//!
//! `/scim/v2/*` is authenticated by a per-org SCIM bearer token (the org is
//! resolved from the token, never the path — so a token can only ever act on
//! its own tenant). Org admins mint/list/revoke those tokens under
//! `/orgs/{org}/scim/tokens`.

pub mod groups;
pub mod tokens;
pub mod users;

use std::sync::Arc;

use axum::{
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use uuid::Uuid;

use crate::auth::scim::store::ScimTokenStore;
use crate::state::AppState;

/// SCIM message schema URIs.
pub const SCHEMA_ERROR: &str = "urn:ietf:params:scim:api:messages:2.0:Error";
pub const SCHEMA_LIST: &str = "urn:ietf:params:scim:api:messages:2.0:ListResponse";
pub const SCHEMA_USER: &str = "urn:ietf:params:scim:schemas:core:2.0:User";
pub const SCHEMA_GROUP: &str = "urn:ietf:params:scim:schemas:core:2.0:Group";
pub const SCHEMA_PATCH: &str = "urn:ietf:params:scim:api:messages:2.0:PatchOp";

/// The authenticated SCIM caller — the single tenant its token authorizes.
#[derive(Debug, Clone, Copy)]
pub struct ScimContext {
    pub org_id: Uuid,
}

/// A SCIM-shaped error (RFC 7644 §3.12): rendered as the SCIM Error envelope.
#[derive(Debug)]
pub struct ScimError {
    pub status: StatusCode,
    pub detail: String,
    pub scim_type: Option<String>,
}

impl ScimError {
    pub fn new(status: StatusCode, detail: impl Into<String>) -> Self {
        Self {
            status,
            detail: detail.into(),
            scim_type: None,
        }
    }
    pub fn unauthorized() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "invalid or missing SCIM token")
    }
    pub fn not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, "resource not found")
    }
    pub fn bad_request(detail: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, detail)
    }
    pub fn internal() -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
    }
    pub fn with_scim_type(mut self, t: impl Into<String>) -> Self {
        self.scim_type = Some(t.into());
        self
    }
}

impl IntoResponse for ScimError {
    fn into_response(self) -> Response {
        let mut body = serde_json::json!({
            "schemas": [SCHEMA_ERROR],
            "status": self.status.as_u16().to_string(),
            "detail": self.detail,
        });
        if let Some(t) = self.scim_type {
            body["scimType"] = serde_json::Value::String(t);
        }
        (self.status, Json(body)).into_response()
    }
}

/// Wrap a set of SCIM resources in a ListResponse envelope.
pub fn list_response(resources: Vec<serde_json::Value>) -> serde_json::Value {
    serde_json::json!({
        "schemas": [SCHEMA_LIST],
        "totalResults": resources.len(),
        "startIndex": 1,
        "itemsPerPage": resources.len(),
        "Resources": resources,
    })
}

/// Authenticate a SCIM request from its `Authorization: Bearer` token and return
/// the tenant it authorizes. This is the only place the SCIM org is decided.
pub async fn authenticate_scim(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<ScimContext, ScimError> {
    let token = bearer_token(headers).ok_or_else(ScimError::unauthorized)?;
    let record = ScimTokenStore::new(&state.db)
        .authenticate(&token)
        .await
        .map_err(|_| ScimError::internal())?
        .ok_or_else(ScimError::unauthorized)?;
    Ok(ScimContext {
        org_id: record.org_id,
    })
}

fn bearer_token(headers: &HeaderMap) -> Option<String> {
    let raw = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    raw.strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))
        .map(|s| s.trim().to_string())
}

/// Build the SCIM + token-admin routes.
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        // SCIM protocol (token-authenticated; org from the token).
        .route(
            "/scim/v2/Users",
            get(users::list_users).post(users::create_user),
        )
        .route(
            "/scim/v2/Users/{id}",
            get(users::get_user)
                .put(users::put_user)
                .patch(users::patch_user)
                .delete(users::delete_user),
        )
        .route("/scim/v2/Groups", get(groups::list_groups))
        .route("/scim/v2/Groups/{id}", get(groups::get_group))
        // Token administration (org admin; standard auth).
        .route(
            "/orgs/{org}/scim/tokens",
            post(tokens::create_token).get(tokens::list_tokens),
        )
        .route(
            "/orgs/{org}/scim/tokens/{token_id}",
            axum::routing::delete(tokens::revoke_token),
        )
}
