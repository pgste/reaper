//! Webhook subscription API endpoints
//!
//! Manages outbound webhook subscriptions for organizations.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    api::error::{ApiError, ApiResult},
    api::orgs::resolve_org,
    db::repositories::{OrganizationRepository, WebhookRepository},
    domain::webhook::{
        CreateWebhookSubscription, UpdateWebhookSubscription, WebhookDeliveryResult,
        WebhookEventType, WebhookSubscription,
    },
    state::AppState,
    webhook::WebhookDeliveryService,
};

/// Build webhook subscription routes (nested under orgs)
pub fn routes() -> OpenApiRouter<Arc<AppState>> {
    OpenApiRouter::new()
        .routes(routes!(list_webhooks, create_webhook))
        .routes(routes!(get_webhook, update_webhook, delete_webhook))
        .routes(routes!(test_webhook))
}

/// Query parameters for listing webhooks
#[derive(Debug, Deserialize, Default)]
pub struct ListWebhooksQuery {
    pub active_only: Option<bool>,
}

/// Response for listing webhooks
#[derive(Debug, Serialize)]
pub struct ListWebhooksResponse {
    pub webhooks: Vec<WebhookSummary>,
    pub total: usize,
}

/// Summary of a webhook subscription (without secret)
#[derive(Debug, Serialize)]
pub struct WebhookSummary {
    pub id: Uuid,
    pub name: String,
    pub url: String,
    pub events: Vec<WebhookEventType>,
    pub is_active: bool,
    pub last_triggered_at: Option<chrono::DateTime<chrono::Utc>>,
    pub failure_count: i32,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<WebhookSubscription> for WebhookSummary {
    fn from(w: WebhookSubscription) -> Self {
        Self {
            id: w.id,
            name: w.name,
            url: w.url,
            events: w.events,
            is_active: w.is_active,
            last_triggered_at: w.last_triggered_at,
            failure_count: w.failure_count,
            created_at: w.created_at,
        }
    }
}

/// Request to create a webhook subscription
#[derive(Debug, Deserialize)]
pub struct CreateWebhookRequest {
    pub name: String,
    pub url: String,
    /// Secret for HMAC signing (optional but recommended)
    pub secret: Option<String>,
    /// Events to subscribe to
    pub events: Vec<WebhookEventType>,
}

/// Request to update a webhook subscription
#[derive(Debug, Deserialize)]
pub struct UpdateWebhookRequest {
    pub name: Option<String>,
    pub url: Option<String>,
    pub secret: Option<String>,
    pub events: Option<Vec<WebhookEventType>>,
    pub is_active: Option<bool>,
}

/// Response for webhook test
#[derive(Debug, Serialize, ToSchema)]
pub struct TestWebhookResponse {
    pub success: bool,
    pub delivery_id: Uuid,
    pub status_code: Option<u16>,
    pub error: Option<String>,
    pub duration_ms: u64,
}

impl From<WebhookDeliveryResult> for TestWebhookResponse {
    fn from(r: WebhookDeliveryResult) -> Self {
        Self {
            success: r.success,
            delivery_id: r.delivery_id,
            status_code: r.status_code,
            error: r.error,
            duration_ms: r.duration_ms,
        }
    }
}

/// List webhook subscriptions for an organization
#[utoipa::path(
    get,
    path = "/orgs/{org}/webhooks",
    tag = "webhooks",
    params(
        ("org" = String, Path, description = "Organization ID or slug")
    ),
    responses(
        (status = 200, description = "List of webhook subscriptions")
    ),
    security(("bearer_jwt" = []))
)]
async fn list_webhooks(
    State(state): State<Arc<AppState>>,
    Path(org): Path<String>,
    Query(query): Query<ListWebhooksQuery>,
) -> ApiResult<Json<ListWebhooksResponse>> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    let webhook_repo = WebhookRepository::new(&state.db);
    let webhooks = webhook_repo
        .list_by_org(organization.id, query.active_only.unwrap_or(false))
        .await?;

    let total = webhooks.len();
    let summaries: Vec<WebhookSummary> = webhooks.into_iter().map(|w| w.into()).collect();

    Ok(Json(ListWebhooksResponse {
        webhooks: summaries,
        total,
    }))
}

/// Create a new webhook subscription
#[utoipa::path(
    post,
    path = "/orgs/{org}/webhooks",
    tag = "webhooks",
    params(
        ("org" = String, Path, description = "Organization ID or slug")
    ),
    responses(
        (status = 201, description = "Webhook subscription created")
    ),
    security(("bearer_jwt" = []))
)]
async fn create_webhook(
    State(state): State<Arc<AppState>>,
    Path(org): Path<String>,
    Json(request): Json<CreateWebhookRequest>,
) -> ApiResult<(StatusCode, Json<WebhookSummary>)> {
    // Validate request
    if request.name.is_empty() {
        return Err(ApiError::BadRequest("Webhook name is required".to_string()));
    }

    if request.url.is_empty() {
        return Err(ApiError::BadRequest("Webhook URL is required".to_string()));
    }

    if request.events.is_empty() {
        return Err(ApiError::BadRequest(
            "At least one event type is required".to_string(),
        ));
    }

    // Validate URL format
    if !request.url.starts_with("https://") && !request.url.starts_with("http://") {
        return Err(ApiError::BadRequest(
            "Webhook URL must be a valid HTTP(S) URL".to_string(),
        ));
    }

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    let webhook_repo = WebhookRepository::new(&state.db);

    // Check for duplicate name
    if webhook_repo
        .get_by_name(organization.id, &request.name)
        .await?
        .is_some()
    {
        return Err(ApiError::Conflict(format!(
            "Webhook with name '{}' already exists",
            request.name
        )));
    }

    let input = CreateWebhookSubscription {
        name: request.name,
        url: request.url,
        secret: request.secret,
        events: request.events,
    };

    let webhook = webhook_repo.create(organization.id, input).await?;

    Ok((StatusCode::CREATED, Json(webhook.into())))
}

/// Get a webhook subscription by ID or name
#[utoipa::path(
    get,
    path = "/orgs/{org}/webhooks/{webhook}",
    tag = "webhooks",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("webhook" = String, Path, description = "Webhook ID or name")
    ),
    responses(
        (status = 200, description = "Webhook subscription")
    ),
    security(("bearer_jwt" = []))
)]
async fn get_webhook(
    State(state): State<Arc<AppState>>,
    Path((org, webhook_ref)): Path<(String, String)>,
) -> ApiResult<Json<WebhookSummary>> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    let webhook_repo = WebhookRepository::new(&state.db);
    let webhook = resolve_webhook(&webhook_repo, organization.id, &webhook_ref).await?;

    Ok(Json(webhook.into()))
}

/// Update a webhook subscription
#[utoipa::path(
    put,
    path = "/orgs/{org}/webhooks/{webhook}",
    tag = "webhooks",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("webhook" = String, Path, description = "Webhook ID or name")
    ),
    responses(
        (status = 200, description = "Webhook subscription updated")
    ),
    security(("bearer_jwt" = []))
)]
async fn update_webhook(
    State(state): State<Arc<AppState>>,
    Path((org, webhook_ref)): Path<(String, String)>,
    Json(request): Json<UpdateWebhookRequest>,
) -> ApiResult<Json<WebhookSummary>> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    let webhook_repo = WebhookRepository::new(&state.db);
    let existing = resolve_webhook(&webhook_repo, organization.id, &webhook_ref).await?;

    // Validate URL if provided
    if let Some(ref url) = request.url {
        if !url.starts_with("https://") && !url.starts_with("http://") {
            return Err(ApiError::BadRequest(
                "Webhook URL must be a valid HTTP(S) URL".to_string(),
            ));
        }
    }

    // Validate events if provided
    if let Some(ref events) = request.events {
        if events.is_empty() {
            return Err(ApiError::BadRequest(
                "At least one event type is required".to_string(),
            ));
        }
    }

    let input = UpdateWebhookSubscription {
        name: request.name,
        url: request.url,
        secret: request.secret,
        events: request.events,
        is_active: request.is_active,
    };

    let updated = webhook_repo
        .update(existing.id, input)
        .await?
        .ok_or_else(|| ApiError::NotFound("Webhook not found after update".to_string()))?;

    Ok(Json(updated.into()))
}

/// Delete a webhook subscription
#[utoipa::path(
    delete,
    path = "/orgs/{org}/webhooks/{webhook}",
    tag = "webhooks",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("webhook" = String, Path, description = "Webhook ID or name")
    ),
    responses(
        (status = 204, description = "Webhook subscription deleted")
    ),
    security(("bearer_jwt" = []))
)]
async fn delete_webhook(
    State(state): State<Arc<AppState>>,
    Path((org, webhook_ref)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    let webhook_repo = WebhookRepository::new(&state.db);
    let existing = resolve_webhook(&webhook_repo, organization.id, &webhook_ref).await?;

    let deleted = webhook_repo.delete(existing.id).await?;

    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound("Webhook not found".to_string()))
    }
}

/// Test a webhook by sending a test payload
#[utoipa::path(
    post,
    path = "/orgs/{org}/webhooks/{webhook}/test",
    tag = "webhooks",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("webhook" = String, Path, description = "Webhook ID or name")
    ),
    responses(
        (status = 200, description = "Webhook test delivery result", body = TestWebhookResponse)
    ),
    security(("bearer_jwt" = []))
)]
async fn test_webhook(
    State(state): State<Arc<AppState>>,
    Path((org, webhook_ref)): Path<(String, String)>,
) -> ApiResult<Json<TestWebhookResponse>> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    let webhook_repo = WebhookRepository::new(&state.db);
    let webhook = resolve_webhook(&webhook_repo, organization.id, &webhook_ref).await?;

    let delivery_service = WebhookDeliveryService::new(state.db.clone());
    let result = delivery_service
        .test_webhook(&webhook, &organization.slug)
        .await;

    Ok(Json(result.into()))
}

/// Resolve webhook by ID or name
async fn resolve_webhook(
    repo: &WebhookRepository<'_>,
    org_id: Uuid,
    webhook_ref: &str,
) -> ApiResult<WebhookSubscription> {
    // Try parsing as UUID first
    if let Ok(id) = Uuid::parse_str(webhook_ref) {
        if let Some(webhook) = repo.get_by_id(id).await? {
            if webhook.org_id == org_id {
                return Ok(webhook);
            }
        }
    }

    // Try by name
    repo.get_by_name(org_id, webhook_ref)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Webhook '{}' not found", webhook_ref)))
}
