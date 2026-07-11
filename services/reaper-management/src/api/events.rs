//! Server-Sent Events (SSE) endpoint
//!
//! Provides real-time event streaming for agents with namespace filtering.

use axum::{
    extract::{Path, Query, State},
    response::sse::{Event, Sse},
};
use futures::stream::Stream;
use serde::Deserialize;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;
use uuid::Uuid;

use crate::{
    api::error::ApiError,
    api::orgs::resolve_org,
    auth::middleware::RequireAuth,
    db::repositories::{NamespaceRepository, OrganizationRepository},
    state::{AppState, ServerEvent},
};
use utoipa_axum::{router::OpenApiRouter, routes};

/// Query parameters for event stream
#[derive(Debug, Deserialize, Default)]
pub struct EventStreamQuery {
    /// Filter by specific namespace IDs (comma-separated)
    pub namespaces: Option<String>,
    /// Include events from child namespaces
    #[serde(default)]
    pub include_children: bool,
}

/// Build SSE routes
pub fn routes() -> OpenApiRouter<Arc<AppState>> {
    OpenApiRouter::new()
        // SSE event stream (org-wide)
        .routes(routes!(events_stream))
        // SSE event stream for agent (filtered by subscriptions)
        .routes(routes!(agent_events_stream))
}

/// SSE event stream for an organization (with optional namespace filtering)
#[utoipa::path(
    get,
    path = "/orgs/{org}/events",
    tag = "events",
    params(
        ("org" = String, Path, description = "Organization ID or slug")
    ),
    responses(
        (status = 200, description = "SSE stream of organization events", content_type = "text/event-stream")
    ),
    security(("bearer_jwt" = []))
)]
async fn events_stream(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Query(query): Query<EventStreamQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    // Verify user belongs to this org
    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot access events for other organizations".to_string(),
        ));
    }

    let org_id = organization.id;

    // Parse namespace filter
    let namespace_filter: Vec<Uuid> = query
        .namespaces
        .map(|ns| {
            ns.split(',')
                .filter_map(|s| Uuid::parse_str(s.trim()).ok())
                .collect()
        })
        .unwrap_or_default();

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
                    // Apply namespace filter if specified
                    if !namespace_filter.is_empty()
                        && !event.matches_subscriptions(&namespace_filter)
                    {
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

/// SSE event stream for an agent (filtered by subscriptions)
#[utoipa::path(
    get,
    path = "/orgs/{org}/agents/{agent_id}/events",
    tag = "events",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("agent_id" = Uuid, Path, description = "Agent ID")
    ),
    responses(
        (status = 200, description = "SSE stream of agent-filtered events", content_type = "text/event-stream")
    ),
    security(("bearer_jwt" = []))
)]
async fn agent_events_stream(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, agent_id)): Path<(String, Uuid)>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    // Verify user belongs to this org
    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot access events for other organizations".to_string(),
        ));
    }

    let org_id = organization.id;

    // Get agent's namespace subscriptions
    let ns_repo = NamespaceRepository::new(&state.db);
    let subscriptions = ns_repo.get_agent_subscriptions(agent_id).await?;
    let subscribed_namespaces: Vec<Uuid> = subscriptions.iter().map(|s| s.namespace_id).collect();

    // Subscribe to event stream
    let rx = state.subscribe_events();
    let stream = BroadcastStream::new(rx);

    // Transform events into SSE format with namespace filtering
    let sse_stream = stream
        .filter_map(move |result| {
            match result {
                Ok(event) => {
                    // Filter events for this organization
                    if !event_matches_org(&event, org_id) {
                        return None;
                    }
                    // Filter by agent's subscriptions (empty subscriptions = all events)
                    if !subscribed_namespaces.is_empty()
                        && !event.matches_subscriptions(&subscribed_namespaces)
                    {
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
fn event_matches_org(event: &ServerEvent, org_id: Uuid) -> bool {
    match event.org_id() {
        Some(eid) => eid == org_id,
        None => true, // Events without org_id (like Ping) go to everyone
    }
}

/// Convert ServerEvent to SSE Event
fn event_to_sse(event: ServerEvent) -> Event {
    match event {
        ServerEvent::PolicyUpdated {
            policy_id,
            org_id,
            namespace_id,
            version,
        } => Event::default()
            .event("policy_updated")
            .json_data(serde_json::json!({
                "policy_id": policy_id,
                "org_id": org_id,
                "namespace_id": namespace_id,
                "version": version
            }))
            .unwrap_or_else(|_| Event::default().data("error")),

        ServerEvent::PolicyDeleted {
            policy_id,
            org_id,
            namespace_id,
        } => Event::default()
            .event("policy_deleted")
            .json_data(serde_json::json!({
                "policy_id": policy_id,
                "org_id": org_id,
                "namespace_id": namespace_id
            }))
            .unwrap_or_else(|_| Event::default().data("error")),

        ServerEvent::BundlePromoted {
            bundle_id,
            org_id,
            namespace_id,
            version,
            download_url,
        } => Event::default()
            .event("bundle_promoted")
            .json_data(serde_json::json!({
                "bundle_id": bundle_id,
                "org_id": org_id,
                "namespace_id": namespace_id,
                "version": version,
                "download_url": download_url
            }))
            .unwrap_or_else(|_| Event::default().data("error")),

        ServerEvent::BundleStaged {
            bundle_id,
            org_id,
            namespace_id,
        } => Event::default()
            .event("bundle_staged")
            .json_data(serde_json::json!({
                "bundle_id": bundle_id,
                "org_id": org_id,
                "namespace_id": namespace_id
            }))
            .unwrap_or_else(|_| Event::default().data("error")),

        ServerEvent::DataRefresh {
            source_id,
            org_id,
            namespace_id,
            source_type,
        } => Event::default()
            .event("data_refresh")
            .json_data(serde_json::json!({
                "source_id": source_id,
                "org_id": org_id,
                "namespace_id": namespace_id,
                "source_type": source_type
            }))
            .unwrap_or_else(|_| Event::default().data("error")),

        ServerEvent::Ping { timestamp } => Event::default()
            .event("ping")
            .json_data(serde_json::json!({
                "timestamp": timestamp.to_rfc3339()
            }))
            .unwrap_or_else(|_| Event::default().data("ping")),

        ServerEvent::RolloutStarted {
            rollout_id,
            bundle_id,
            org_id,
            namespace_id,
        } => Event::default()
            .event("rollout_started")
            .json_data(serde_json::json!({
                "rollout_id": rollout_id,
                "bundle_id": bundle_id,
                "org_id": org_id,
                "namespace_id": namespace_id
            }))
            .unwrap_or_else(|_| Event::default().data("error")),

        ServerEvent::RolloutWaveCompleted {
            rollout_id,
            wave_number,
            org_id,
            namespace_id,
        } => Event::default()
            .event("rollout_wave_completed")
            .json_data(serde_json::json!({
                "rollout_id": rollout_id,
                "wave_number": wave_number,
                "org_id": org_id,
                "namespace_id": namespace_id
            }))
            .unwrap_or_else(|_| Event::default().data("error")),

        ServerEvent::RolloutCompleted {
            rollout_id,
            bundle_id,
            org_id,
            namespace_id,
            success,
        } => Event::default()
            .event("rollout_completed")
            .json_data(serde_json::json!({
                "rollout_id": rollout_id,
                "bundle_id": bundle_id,
                "org_id": org_id,
                "namespace_id": namespace_id,
                "success": success
            }))
            .unwrap_or_else(|_| Event::default().data("error")),

        ServerEvent::DatastorePublished {
            datastore_id,
            org_id,
            namespace_id,
            version,
            checksum,
        } => Event::default()
            .event("datastore_published")
            .json_data(serde_json::json!({
                "datastore_id": datastore_id,
                "org_id": org_id,
                "namespace_id": namespace_id,
                "version": version,
                "checksum": checksum
            }))
            .unwrap_or_else(|_| Event::default().data("error")),

        ServerEvent::SyncStarted {
            source_id,
            source_name,
            org_id,
            namespace_id,
        } => Event::default()
            .event("sync_started")
            .json_data(serde_json::json!({
                "source_id": source_id,
                "source_name": source_name,
                "org_id": org_id,
                "namespace_id": namespace_id
            }))
            .unwrap_or_else(|_| Event::default().data("error")),

        ServerEvent::SyncCompleted {
            source_id,
            source_name,
            org_id,
            namespace_id,
            policies_updated,
            duration_ms,
        } => Event::default()
            .event("sync_completed")
            .json_data(serde_json::json!({
                "source_id": source_id,
                "source_name": source_name,
                "org_id": org_id,
                "namespace_id": namespace_id,
                "policies_updated": policies_updated,
                "duration_ms": duration_ms
            }))
            .unwrap_or_else(|_| Event::default().data("error")),

        ServerEvent::SyncFailed {
            source_id,
            source_name,
            org_id,
            namespace_id,
            error,
        } => Event::default()
            .event("sync_failed")
            .json_data(serde_json::json!({
                "source_id": source_id,
                "source_name": source_name,
                "org_id": org_id,
                "namespace_id": namespace_id,
                "error": error
            }))
            .unwrap_or_else(|_| Event::default().data("error")),

        ServerEvent::AgentRegistered {
            agent_id,
            agent_name,
            org_id,
            namespace_id,
        } => Event::default()
            .event("agent_registered")
            .json_data(serde_json::json!({
                "agent_id": agent_id,
                "agent_name": agent_name,
                "org_id": org_id,
                "namespace_id": namespace_id
            }))
            .unwrap_or_else(|_| Event::default().data("error")),

        ServerEvent::AgentUnhealthy {
            agent_id,
            agent_name,
            org_id,
            namespace_id,
            last_seen,
        } => Event::default()
            .event("agent_unhealthy")
            .json_data(serde_json::json!({
                "agent_id": agent_id,
                "agent_name": agent_name,
                "org_id": org_id,
                "namespace_id": namespace_id,
                "last_seen": last_seen.to_rfc3339()
            }))
            .unwrap_or_else(|_| Event::default().data("error")),

        ServerEvent::AgentHealthy {
            agent_id,
            agent_name,
            org_id,
            namespace_id,
        } => Event::default()
            .event("agent_healthy")
            .json_data(serde_json::json!({
                "agent_id": agent_id,
                "agent_name": agent_name,
                "org_id": org_id,
                "namespace_id": namespace_id
            }))
            .unwrap_or_else(|_| Event::default().data("error")),

        ServerEvent::AgentDataStale {
            agent_id,
            agent_name,
            org_id,
            namespace_id,
            data_version,
            data_applied_seq,
        } => Event::default()
            .event("agent_data_stale")
            .json_data(serde_json::json!({
                "agent_id": agent_id,
                "agent_name": agent_name,
                "org_id": org_id,
                "namespace_id": namespace_id,
                "data_version": data_version,
                "data_applied_seq": data_applied_seq
            }))
            .unwrap_or_else(|_| Event::default().data("error")),

        ServerEvent::AgentDataFresh {
            agent_id,
            agent_name,
            org_id,
            namespace_id,
            data_version,
        } => Event::default()
            .event("agent_data_fresh")
            .json_data(serde_json::json!({
                "agent_id": agent_id,
                "agent_name": agent_name,
                "org_id": org_id,
                "namespace_id": namespace_id,
                "data_version": data_version
            }))
            .unwrap_or_else(|_| Event::default().data("error")),
    }
}
