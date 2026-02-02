//! Client certificate (mTLS) management handlers.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    api::error::{ApiError, ApiResult},
    api::orgs::resolve_org,
    auth::{
        middleware::RequireAuth,
        mtls::{ClientCertificateRepository, RegisterCertificate},
        scopes::Scope,
    },
    db::repositories::OrganizationRepository,
    state::AppState,
};

use super::types::{
    BindCertificateRequest, CertificateSummary, ListCertificatesResponse,
    RegisterCertificateRequest, RevokeCertificateRequest,
};

/// List client certificates for an organization
pub async fn list_certificates(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
) -> ApiResult<Json<ListCertificatesResponse>> {
    // Require org admin permission
    if !user.has_permission(Scope::OrgAdmin) && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Missing org:admin scope".to_string(),
        ));
    }

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    // Verify user belongs to this org
    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot access certificates for other organizations".to_string(),
        ));
    }

    let cert_repo = ClientCertificateRepository::new(&state.db);
    let certs = cert_repo.list_by_org(organization.id).await?;

    let summaries: Vec<CertificateSummary> = certs.into_iter().map(|c| c.into()).collect();

    Ok(Json(ListCertificatesResponse {
        certificates: summaries,
    }))
}

/// Register a new client certificate
pub async fn register_certificate(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Json(request): Json<RegisterCertificateRequest>,
) -> ApiResult<(StatusCode, Json<CertificateSummary>)> {
    // Require org admin permission
    if !user.has_permission(Scope::OrgAdmin) && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Missing org:admin scope".to_string(),
        ));
    }

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    // Verify user belongs to this org
    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot register certificates for other organizations".to_string(),
        ));
    }

    // Validate fingerprint format (should be hex-encoded SHA-256)
    if request.fingerprint.len() != 64 || !request.fingerprint.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ApiError::BadRequest(
            "Fingerprint must be a 64-character hex-encoded SHA-256 hash".to_string(),
        ));
    }

    let cert_repo = ClientCertificateRepository::new(&state.db);

    // Check if fingerprint already exists
    if cert_repo.get_by_fingerprint(&request.fingerprint).await?.is_some() {
        return Err(ApiError::Conflict(
            "Certificate with this fingerprint already registered".to_string(),
        ));
    }

    let input = RegisterCertificate {
        fingerprint: request.fingerprint,
        subject: request.subject,
        issuer: request.issuer,
        not_before: request.not_before,
        not_after: request.not_after,
        agent_id: request.agent_id,
    };

    let cert = cert_repo.create(organization.id, input).await?;

    Ok((StatusCode::CREATED, Json(cert.into())))
}

/// Get a client certificate by ID
pub async fn get_certificate(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, cert_id)): Path<(String, Uuid)>,
) -> ApiResult<Json<CertificateSummary>> {
    // Require org admin permission
    if !user.has_permission(Scope::OrgAdmin) && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Missing org:admin scope".to_string(),
        ));
    }

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    // Verify user belongs to this org
    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot access certificates for other organizations".to_string(),
        ));
    }

    let cert_repo = ClientCertificateRepository::new(&state.db);
    let cert = cert_repo
        .get_by_id(cert_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("Certificate not found".to_string()))?;

    // Verify cert belongs to this org
    if cert.org_id != organization.id {
        return Err(ApiError::NotFound("Certificate not found".to_string()));
    }

    Ok(Json(cert.into()))
}

/// Delete a client certificate
pub async fn delete_certificate(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, cert_id)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    // Require org admin permission
    if !user.has_permission(Scope::OrgAdmin) && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Missing org:admin scope".to_string(),
        ));
    }

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    // Verify user belongs to this org
    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot delete certificates for other organizations".to_string(),
        ));
    }

    let cert_repo = ClientCertificateRepository::new(&state.db);

    // Verify cert exists and belongs to this org
    let cert = cert_repo
        .get_by_id(cert_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("Certificate not found".to_string()))?;

    if cert.org_id != organization.id {
        return Err(ApiError::NotFound("Certificate not found".to_string()));
    }

    cert_repo.delete(cert_id).await?;

    Ok(StatusCode::NO_CONTENT)
}

/// Revoke a client certificate
pub async fn revoke_certificate(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, cert_id)): Path<(String, Uuid)>,
    Json(request): Json<RevokeCertificateRequest>,
) -> ApiResult<StatusCode> {
    // Require org admin permission
    if !user.has_permission(Scope::OrgAdmin) && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Missing org:admin scope".to_string(),
        ));
    }

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    // Verify user belongs to this org
    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot revoke certificates for other organizations".to_string(),
        ));
    }

    let cert_repo = ClientCertificateRepository::new(&state.db);

    // Verify cert exists and belongs to this org
    let cert = cert_repo
        .get_by_id(cert_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("Certificate not found".to_string()))?;

    if cert.org_id != organization.id {
        return Err(ApiError::NotFound("Certificate not found".to_string()));
    }

    if cert.is_revoked {
        return Err(ApiError::Conflict("Certificate is already revoked".to_string()));
    }

    cert_repo.revoke(cert_id, request.reason.as_deref()).await?;

    Ok(StatusCode::NO_CONTENT)
}

/// Bind a certificate to an agent
pub async fn bind_certificate(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, cert_id)): Path<(String, Uuid)>,
    Json(request): Json<BindCertificateRequest>,
) -> ApiResult<StatusCode> {
    // Require org admin permission
    if !user.has_permission(Scope::OrgAdmin) && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Missing org:admin scope".to_string(),
        ));
    }

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    // Verify user belongs to this org
    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot modify certificates for other organizations".to_string(),
        ));
    }

    let cert_repo = ClientCertificateRepository::new(&state.db);

    // Verify cert exists and belongs to this org
    let cert = cert_repo
        .get_by_id(cert_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("Certificate not found".to_string()))?;

    if cert.org_id != organization.id {
        return Err(ApiError::NotFound("Certificate not found".to_string()));
    }

    // Verify agent exists and belongs to this org
    let agent_repo = crate::db::repositories::AgentRepository::new(&state.db);
    let agent = agent_repo
        .get_by_id(request.agent_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("Agent not found".to_string()))?;

    if agent.org_id != organization.id {
        return Err(ApiError::NotFound("Agent not found".to_string()));
    }

    cert_repo.bind_to_agent(cert_id, request.agent_id).await?;

    Ok(StatusCode::NO_CONTENT)
}

/// Unbind a certificate from its agent
pub async fn unbind_certificate(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, cert_id)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    // Require org admin permission
    if !user.has_permission(Scope::OrgAdmin) && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Missing org:admin scope".to_string(),
        ));
    }

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    // Verify user belongs to this org
    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot modify certificates for other organizations".to_string(),
        ));
    }

    let cert_repo = ClientCertificateRepository::new(&state.db);

    // Verify cert exists and belongs to this org
    let cert = cert_repo
        .get_by_id(cert_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("Certificate not found".to_string()))?;

    if cert.org_id != organization.id {
        return Err(ApiError::NotFound("Certificate not found".to_string()));
    }

    cert_repo.unbind_from_agent(cert_id).await?;

    Ok(StatusCode::NO_CONTENT)
}
