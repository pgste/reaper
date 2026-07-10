//! Webhook API endpoints
//!
//! Provides endpoints for receiving webhook notifications from external systems.
//! Used primarily for BundleUrl sources to trigger bundle fetches.

use axum::{
    body::Bytes,
    extract::{Path, State},
    http::HeaderMap,
    response::Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, info};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    api::error::{ApiError, ApiResult},
    db::repositories::PolicySourceRepository,
    domain::source::SourceType,
    state::{AppState, ServerEvent},
    sync::BundleUrlSyncer,
};

/// Build webhook routes
pub fn routes() -> OpenApiRouter<Arc<AppState>> {
    OpenApiRouter::new()
        .routes(routes!(bundle_update_webhook))
        .routes(routes!(bundle_update_webhook_with_source))
}

/// Request body for bundle update webhook
#[derive(Debug, Deserialize, ToSchema)]
pub struct BundleUpdateRequest {
    /// Source ID (optional if included in URL)
    pub source_id: Option<Uuid>,
    /// URL to fetch the bundle from
    pub bundle_url: String,
    /// Bundle version
    #[serde(default)]
    pub version: Option<String>,
    /// Checksum of the bundle (format: "sha256:abc123" or just "abc123")
    #[serde(default)]
    pub checksum: Option<String>,
    /// Bundle format (rbb or rpp, defaults to rbb)
    #[serde(default)]
    pub format: Option<String>,
    /// Optional metadata
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

/// Response for bundle update webhook
#[derive(Debug, Serialize, ToSchema)]
pub struct BundleUpdateResponse {
    pub success: bool,
    pub message: String,
    pub bundle_id: Option<Uuid>,
    pub version: Option<String>,
    pub checksum: Option<String>,
    pub size_bytes: Option<usize>,
}

/// Handle bundle update webhook (source_id in body)
#[utoipa::path(
    post,
    path = "/webhooks/bundle-update",
    tag = "webhooks",
    request_body = BundleUpdateRequest,
    responses(
        (status = 200, description = "Bundle fetched and stored", body = BundleUpdateResponse)
    )
)]
async fn bundle_update_webhook(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Json<BundleUpdateResponse>> {
    // Parse the request body
    let request: BundleUpdateRequest = serde_json::from_slice(&body)
        .map_err(|e| ApiError::Validation(format!("Invalid JSON body: {}", e)))?;

    let source_id = request.source_id.ok_or_else(|| {
        ApiError::Validation("source_id is required when not in URL path".to_string())
    })?;

    process_bundle_webhook(state, headers, &body, source_id, request).await
}

/// Handle bundle update webhook (source_id in URL)
#[utoipa::path(
    post,
    path = "/webhooks/bundle-update/{source_id}",
    tag = "webhooks",
    params(
        ("source_id" = Uuid, Path, description = "Source ID")
    ),
    request_body = BundleUpdateRequest,
    responses(
        (status = 200, description = "Bundle fetched and stored", body = BundleUpdateResponse)
    )
)]
async fn bundle_update_webhook_with_source(
    State(state): State<Arc<AppState>>,
    Path(source_id): Path<Uuid>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Json<BundleUpdateResponse>> {
    // Parse the request body
    let request: BundleUpdateRequest = serde_json::from_slice(&body)
        .map_err(|e| ApiError::Validation(format!("Invalid JSON body: {}", e)))?;

    process_bundle_webhook(state, headers, &body, source_id, request).await
}

/// Process the bundle webhook
async fn process_bundle_webhook(
    state: Arc<AppState>,
    headers: HeaderMap,
    body: &[u8],
    source_id: Uuid,
    request: BundleUpdateRequest,
) -> ApiResult<Json<BundleUpdateResponse>> {
    debug!(
        source_id = %source_id,
        bundle_url = %request.bundle_url,
        "Processing bundle update webhook"
    );

    // Look up the source
    let source_repo = PolicySourceRepository::new(&state.db);
    let source = source_repo
        .get_by_id(source_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("Source not found".to_string()))?;

    // Verify this is a BundleUrl source
    if source.source_type != SourceType::BundleUrl {
        return Err(ApiError::Validation(format!(
            "Source {} is not a BundleUrl source (type: {})",
            source_id, source.source_type
        )));
    }

    // Get the BundleUrl config
    let config = source
        .bundle_url_config()
        .ok_or_else(|| ApiError::Internal("Failed to parse BundleUrl config".to_string()))?;

    // Validate webhook signature if configured
    if config.webhook_secret.is_some() {
        let signature = headers
            .get("x-webhook-signature")
            .or_else(|| headers.get("x-hub-signature-256"))
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| ApiError::Unauthorized("Missing webhook signature".to_string()))?;

        let syncer = BundleUrlSyncer::default();
        let valid = syncer
            .validate_webhook_signature(&config, body, signature)
            .map_err(|e| ApiError::Unauthorized(format!("Signature validation failed: {}", e)))?;

        if !valid {
            return Err(ApiError::Unauthorized(
                "Invalid webhook signature".to_string(),
            ));
        }
    }

    // Create a syncer and fetch the bundle
    let syncer = BundleUrlSyncer::new(&state.config.sync.bundle_storage_path);

    let bundle = syncer
        .fetch_bundle(
            &source,
            &request.bundle_url,
            request.version.as_deref(),
            request.checksum.as_deref(),
        )
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to fetch bundle: {}", e)))?;

    // Store the bundle
    let bundle_path = syncer
        .store_bundle(source_id, &bundle)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to store bundle: {}", e)))?;

    info!(
        source_id = %source_id,
        bundle_path = ?bundle_path,
        version = ?bundle.version,
        size_bytes = bundle.size_bytes,
        "Bundle stored successfully"
    );

    // Broadcast event to notify agents
    let bundle_id = Uuid::new_v4(); // Generate a bundle ID for this update
    state.broadcast_event(ServerEvent::BundlePromoted {
        bundle_id,
        org_id: source.org_id,
        namespace_id: None, // Bundle URL sources are org-wide by default
        version: bundle
            .version
            .clone()
            .unwrap_or_else(|| "latest".to_string()),
        download_url: bundle_path.to_string_lossy().to_string(),
    });

    Ok(Json(BundleUpdateResponse {
        success: true,
        message: "Bundle fetched and stored successfully".to_string(),
        bundle_id: Some(bundle_id),
        version: bundle.version,
        checksum: Some(bundle.checksum),
        size_bytes: Some(bundle.size_bytes),
    }))
}

/// Generic webhook for S3 event notifications
#[derive(Debug, Deserialize)]
pub struct S3EventNotification {
    /// S3 event records
    #[serde(rename = "Records")]
    pub records: Vec<S3EventRecord>,
}

#[derive(Debug, Deserialize)]
pub struct S3EventRecord {
    /// Event source (e.g., "aws:s3")
    #[serde(rename = "eventSource")]
    pub event_source: Option<String>,
    /// Event name (e.g., "ObjectCreated:Put")
    #[serde(rename = "eventName")]
    pub event_name: Option<String>,
    /// S3 information
    pub s3: Option<S3Info>,
}

#[derive(Debug, Deserialize)]
pub struct S3Info {
    /// Bucket information
    pub bucket: Option<S3Bucket>,
    /// Object information
    pub object: Option<S3Object>,
}

#[derive(Debug, Deserialize)]
pub struct S3Bucket {
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct S3Object {
    pub key: Option<String>,
    #[serde(rename = "eTag")]
    pub etag: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_bundle_update_request() {
        let json = r#"{
            "source_id": "550e8400-e29b-41d4-a716-446655440000",
            "bundle_url": "https://example.com/bundle.rbb",
            "version": "1.0.0",
            "checksum": "sha256:abc123"
        }"#;

        let request: BundleUpdateRequest = serde_json::from_str(json).unwrap();
        assert_eq!(
            request.source_id,
            Some(Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap())
        );
        assert_eq!(request.bundle_url, "https://example.com/bundle.rbb");
        assert_eq!(request.version, Some("1.0.0".to_string()));
        assert_eq!(request.checksum, Some("sha256:abc123".to_string()));
    }

    #[test]
    fn test_parse_bundle_update_minimal() {
        let json = r#"{
            "bundle_url": "https://example.com/bundle.rpp"
        }"#;

        let request: BundleUpdateRequest = serde_json::from_str(json).unwrap();
        assert!(request.source_id.is_none());
        assert_eq!(request.bundle_url, "https://example.com/bundle.rpp");
        assert!(request.version.is_none());
        assert!(request.checksum.is_none());
    }

    #[test]
    fn test_parse_s3_event() {
        let json = r#"{
            "Records": [{
                "eventSource": "aws:s3",
                "eventName": "ObjectCreated:Put",
                "s3": {
                    "bucket": {"name": "my-bucket"},
                    "object": {"key": "policies/main.reap", "eTag": "abc123"}
                }
            }]
        }"#;

        let event: S3EventNotification = serde_json::from_str(json).unwrap();
        assert_eq!(event.records.len(), 1);
        assert_eq!(event.records[0].event_source, Some("aws:s3".to_string()));
    }
}
