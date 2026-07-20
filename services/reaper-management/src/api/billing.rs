//! Billing API endpoints
//!
//! Provides endpoints for managing subscriptions and billing.

use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::{
    api::error::{ApiError, ApiResult},
    api::orgs::authorize_org,
    auth::middleware::RequireAuth,
    auth::scopes::Scope,
    billing::{BillingError, BillingService},
    domain::billing::{
        BillingSummary, CheckoutSessionResponse, PlanLimits, PlanTier, PortalSessionResponse,
        UsageMetrics,
    },
    state::AppState,
};

/// Build billing routes
pub fn routes() -> OpenApiRouter<Arc<AppState>> {
    OpenApiRouter::new()
        .routes(routes!(get_billing_summary))
        .routes(routes!(create_checkout))
        .routes(routes!(create_portal))
        .routes(routes!(list_plans))
        .routes(routes!(stripe_webhook))
}

/// Response for billing summary
#[derive(Debug, Serialize)]
pub struct BillingSummaryResponse {
    pub plan_tier: PlanTier,
    pub limits: PlanLimits,
    /// Real usage counts (round-2 E4): active agents, policies, bundles, users.
    pub usage: UsageMetrics,
    pub is_within_limits: bool,
    pub exceeded_limits: Vec<String>,
    pub billing_enabled: bool,
}

impl From<BillingSummary> for BillingSummaryResponse {
    fn from(s: BillingSummary) -> Self {
        Self {
            // The effective plan tier drives the limits, so report it from there
            // (plans persist in org settings now, not a subscription row).
            plan_tier: s.limits.tier,
            limits: s.limits,
            usage: s.usage,
            is_within_limits: s.within_limits,
            exceeded_limits: s.exceeded_limits,
            billing_enabled: true,
        }
    }
}

/// Request to create checkout session
#[derive(Debug, Deserialize)]
pub struct CreateCheckoutRequest {
    pub plan_tier: PlanTier,
    #[serde(default)]
    pub yearly: bool,
    pub success_url: String,
    pub cancel_url: String,
}

/// Request to create portal session
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreatePortalRequest {
    pub return_url: String,
}

/// Plan information
#[derive(Debug, Serialize)]
pub struct PlanInfo {
    pub tier: PlanTier,
    pub name: String,
    pub description: String,
    pub limits: PlanLimits,
    pub price_monthly_cents: Option<i64>,
    pub price_yearly_cents: Option<i64>,
}

/// Get billing summary for an organization
#[utoipa::path(
    get,
    path = "/orgs/{org}/billing",
    tag = "billing",
    params(
        ("org" = String, Path, description = "Organization ID or slug")
    ),
    responses((status = 200, description = "Billing summary for the organization")),
    security(("bearer_jwt" = []))
)]
async fn get_billing_summary(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
) -> ApiResult<Json<BillingSummaryResponse>> {
    let organization = authorize_org(&state, &user, &org, &[Scope::OrgAdmin]).await?;

    let billing_service = BillingService::disabled(state.db.clone());
    let summary = billing_service
        .get_billing_summary(&organization)
        .await
        .map_err(|e| ApiError::Internal(format!("billing summary: {e}")))?;

    let mut response: BillingSummaryResponse = summary.into();
    response.billing_enabled = billing_service.is_enabled();

    Ok(Json(response))
}

/// Create a checkout session for upgrading
#[utoipa::path(
    post,
    path = "/orgs/{org}/billing/checkout",
    tag = "billing",
    params(
        ("org" = String, Path, description = "Organization ID or slug")
    ),
    responses((status = 200, description = "Stripe checkout session created")),
    security(("bearer_jwt" = []))
)]
async fn create_checkout(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Json(request): Json<CreateCheckoutRequest>,
) -> ApiResult<Json<CheckoutSessionResponse>> {
    let organization = authorize_org(&state, &user, &org, &[Scope::OrgAdmin]).await?;

    // Validate plan tier
    if request.plan_tier == PlanTier::Free {
        return Err(ApiError::BadRequest(
            "Cannot create checkout for free tier".to_string(),
        ));
    }

    // Get or create billing service with config
    let billing_service = match crate::billing::BillingConfig::from_env() {
        Ok(config) => BillingService::new(state.db.clone(), config),
        Err(_) => {
            return Err(ApiError::Internal(
                "Billing not configured. Set STRIPE_SECRET_KEY and STRIPE_WEBHOOK_SECRET."
                    .to_string(),
            ))
        }
    };

    // Get or create customer
    let customer_id = billing_service
        .get_or_create_customer(
            organization.id,
            &organization.name,
            "billing@example.com", // In production, use org owner's email
        )
        .await
        .map_err(billing_error_to_api)?;

    // Create checkout session
    let session = billing_service
        .create_checkout_session(
            &customer_id,
            organization.id,
            request.plan_tier,
            request.yearly,
            &request.success_url,
            &request.cancel_url,
        )
        .await
        .map_err(billing_error_to_api)?;

    Ok(Json(session))
}

/// Create a billing portal session
#[utoipa::path(
    post,
    path = "/orgs/{org}/billing/portal",
    tag = "billing",
    params(
        ("org" = String, Path, description = "Organization ID or slug")
    ),
    request_body = CreatePortalRequest,
    responses((status = 200, description = "Stripe billing portal session created")),
    security(("bearer_jwt" = []))
)]
async fn create_portal(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Json(request): Json<CreatePortalRequest>,
) -> ApiResult<Json<PortalSessionResponse>> {
    let organization = authorize_org(&state, &user, &org, &[Scope::OrgAdmin]).await?;

    // Get billing service with config
    let billing_service = match crate::billing::BillingConfig::from_env() {
        Ok(config) => BillingService::new(state.db.clone(), config),
        Err(_) => return Err(ApiError::Internal("Billing not configured".to_string())),
    };

    // For portal, we need existing customer ID
    // In production, retrieve from database
    let customer_id = format!("cus_placeholder_{}", organization.id);

    let session = billing_service
        .create_portal_session(&customer_id, &request.return_url)
        .await
        .map_err(billing_error_to_api)?;

    Ok(Json(session))
}

/// List available plans (static catalog; any authenticated caller)
#[utoipa::path(
    get,
    path = "/orgs/{org}/billing/plans",
    tag = "billing",
    params(
        ("org" = String, Path, description = "Organization ID or slug")
    ),
    responses((status = 200, description = "Static catalog of available plans")),
    security(("bearer_jwt" = []))
)]
async fn list_plans(
    State(_state): State<Arc<AppState>>,
    RequireAuth(_user): RequireAuth,
    Path(_org): Path<String>,
) -> ApiResult<Json<Vec<PlanInfo>>> {
    let plans = vec![
        PlanInfo {
            tier: PlanTier::Free,
            name: "Free".to_string(),
            description: "For individuals and small projects".to_string(),
            limits: PlanLimits::free(),
            price_monthly_cents: None,
            price_yearly_cents: None,
        },
        PlanInfo {
            tier: PlanTier::Starter,
            name: "Starter".to_string(),
            description: "For small teams getting started".to_string(),
            limits: PlanLimits::starter(),
            price_monthly_cents: Some(2900), // $29/month
            price_yearly_cents: Some(29000), // $290/year
        },
        PlanInfo {
            tier: PlanTier::Professional,
            name: "Professional".to_string(),
            description: "For growing teams with advanced needs".to_string(),
            limits: PlanLimits::professional(),
            price_monthly_cents: Some(9900), // $99/month
            price_yearly_cents: Some(99000), // $990/year
        },
        PlanInfo {
            tier: PlanTier::Enterprise,
            name: "Enterprise".to_string(),
            description: "For large organizations with custom requirements".to_string(),
            limits: PlanLimits::enterprise(),
            price_monthly_cents: None, // Contact sales
            price_yearly_cents: None,
        },
    ];

    Ok(Json(plans))
}

/// Handle Stripe webhook
#[utoipa::path(
    post,
    path = "/webhooks/stripe",
    tag = "billing",
    responses(
        (status = 200, description = "Webhook processed"),
        (status = 400, description = "Missing or invalid signature")
    )
)]
async fn stripe_webhook(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<StatusCode> {
    let signature = headers
        .get("stripe-signature")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| ApiError::BadRequest("Missing Stripe signature".to_string()))?;

    let payload = std::str::from_utf8(&body)
        .map_err(|_| ApiError::BadRequest("Invalid UTF-8 in webhook body".to_string()))?;

    // Get billing service with config
    let billing_service = match crate::billing::BillingConfig::from_env() {
        Ok(config) => BillingService::new(state.db.clone(), config),
        Err(_) => return Err(ApiError::Internal("Billing not configured".to_string())),
    };

    billing_service
        .handle_webhook_event(payload, signature)
        .await
        .map_err(billing_error_to_api)?;

    Ok(StatusCode::OK)
}

/// Convert billing error to API error
fn billing_error_to_api(e: BillingError) -> ApiError {
    match e {
        BillingError::NotFound => ApiError::NotFound("Subscription not found".to_string()),
        BillingError::InvalidPlan(msg) => ApiError::BadRequest(msg),
        BillingError::NotConfigured => ApiError::Internal("Billing not configured".to_string()),
        BillingError::Config(msg) => ApiError::Internal(format!("Billing config error: {}", msg)),
        BillingError::Database(e) => ApiError::Internal(format!("Database error: {}", e)),
        BillingError::Stripe(msg) => ApiError::Internal(format!("Stripe error: {}", msg)),
        BillingError::NotImplemented(msg) => ApiError::NotImplemented(msg),
    }
}
