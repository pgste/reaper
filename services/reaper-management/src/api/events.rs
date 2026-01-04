//! Server-Sent Events (SSE) endpoint
//!
//! Provides real-time event streaming for agents.

use axum::{
    extract::{Path, State},
    response::sse::{Event, Sse},
    routing::get,
    Router,
};
use futures::stream::Stream;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

use crate::{
    api::error::ApiError,
    api::orgs::resolve_org,
    auth::middleware::RequireAuth,
    db::repositories::OrganizationRepository,
    state::{AppState, ServerEvent},
};

/// Build SSE routes
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        // SSE event stream
        .route("/api/v1/orgs/{org}/events", get(events_stream))
}

/// SSE event stream for an organization
async fn events_stream(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    // Verify user belongs to this org
    if user.org_id != organization.id && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin]) {
        return Err(ApiError::Forbidden(
            "Cannot access events for other organizations".to_string(),
        ));
    }

    let org_id = organization.id;

    // Subscribe to event stream
    let rx = state.subscribe_events();
    let stream = BroadcastStream::new(rx);

    // Transform events into SSE format
    let sse_stream = stream
        .filter_map(move |result| {
            match result {
                Ok(event) => {
                    // Filter events for this organization
                    if !event_matches_org(&event, org_id) {
                        return None;
                    }
                    Some(event_to_sse(event))
                }
                Err(_) => None, // Ignore lagged errors
            }
        })
        .map(Ok);

    Ok(Sse::new(sse_stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(Duration::from_secs(30))
            .text("ping"),
    ))
}

/// Check if event belongs to the specified organization
fn event_matches_org(event: &ServerEvent, org_id: uuid::Uuid) -> bool {
    match event {
        ServerEvent::PolicyUpdated { org_id: eid, .. } => *eid == org_id,
        ServerEvent::PolicyDeleted { org_id: eid, .. } => *eid == org_id,
        ServerEvent::BundlePromoted { org_id: eid, .. } => *eid == org_id,
        ServerEvent::BundleStaged { org_id: eid, .. } => *eid == org_id,
        ServerEvent::DataRefresh { org_id: eid, .. } => *eid == org_id,
        ServerEvent::Ping { .. } => true, // Ping goes to everyone
    }
}

/// Convert ServerEvent to SSE Event
fn event_to_sse(event: ServerEvent) -> Event {
    match event {
        ServerEvent::PolicyUpdated {
            policy_id,
            org_id,
            version,
        } => Event::default()
            .event("policy_updated")
            .json_data(serde_json::json!({
                "policy_id": policy_id,
                "org_id": org_id,
                "version": version
            }))
            .unwrap_or_else(|_| Event::default().data("error")),

        ServerEvent::PolicyDeleted { policy_id, org_id } => Event::default()
            .event("policy_deleted")
            .json_data(serde_json::json!({
                "policy_id": policy_id,
                "org_id": org_id
            }))
            .unwrap_or_else(|_| Event::default().data("error")),

        ServerEvent::BundlePromoted {
            bundle_id,
            org_id,
            version,
            download_url,
        } => Event::default()
            .event("bundle_promoted")
            .json_data(serde_json::json!({
                "bundle_id": bundle_id,
                "org_id": org_id,
                "version": version,
                "download_url": download_url
            }))
            .unwrap_or_else(|_| Event::default().data("error")),

        ServerEvent::BundleStaged { bundle_id, org_id } => Event::default()
            .event("bundle_staged")
            .json_data(serde_json::json!({
                "bundle_id": bundle_id,
                "org_id": org_id
            }))
            .unwrap_or_else(|_| Event::default().data("error")),

        ServerEvent::DataRefresh {
            source_id,
            org_id,
            source_type,
        } => Event::default()
            .event("data_refresh")
            .json_data(serde_json::json!({
                "source_id": source_id,
                "org_id": org_id,
                "source_type": source_type
            }))
            .unwrap_or_else(|_| Event::default().data("error")),

        ServerEvent::Ping { timestamp } => Event::default()
            .event("ping")
            .json_data(serde_json::json!({
                "timestamp": timestamp.to_rfc3339()
            }))
            .unwrap_or_else(|_| Event::default().data("ping")),
    }
}
