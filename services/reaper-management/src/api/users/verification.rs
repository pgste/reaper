//! Email verification handlers.

use axum::{extract::State, http::StatusCode, response::Json};
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    api::error::{ApiError, ApiResult},
    audit::{actions, ActorType, AuditEntry, ResourceType},
    auth::{
        middleware::RequireAuth,
        users::{EmailVerificationRepository, EmailVerificationToken, UserRepository},
    },
    state::AppState,
};

use super::types::{VerifyEmailRequest, VerifyEmailResponse};

/// Verify email with token
pub async fn verify_email(
    State(state): State<Arc<AppState>>,
    Json(request): Json<VerifyEmailRequest>,
) -> ApiResult<Json<VerifyEmailResponse>> {
    let verification_repo = EmailVerificationRepository::new(&state.db);
    let user_repo = UserRepository::new(&state.db);

    // Find and validate token
    let token = verification_repo
        .find_by_token(&request.token)
        .await?
        .ok_or_else(|| ApiError::BadRequest("Invalid verification token".to_string()))?;

    if !token.is_valid() {
        return Err(ApiError::BadRequest(
            "Verification token has expired".to_string(),
        ));
    }

    // Get user and verify they exist
    let user = user_repo
        .find_by_id(token.user_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("User not found".to_string()))?;

    // Check if already verified
    if user.email_verified {
        // Delete the token since it's no longer needed
        verification_repo.delete(token.id).await?;
        return Ok(Json(VerifyEmailResponse {
            verified: true,
            message: "Email already verified".to_string(),
        }));
    }

    // Mark email as verified
    user_repo.verify_email(token.user_id).await?;

    // Delete the verification token
    verification_repo.delete(token.id).await?;

    // Audit log
    AuditEntry::builder(
        actions::USER_EMAIL_VERIFY,
        ActorType::User,
        token.user_id.to_string(),
    )
    .resource(ResourceType::User, token.user_id.to_string())
    .details(serde_json::json!({
        "email": user.email
    }))
    .log(&state.db)
    .await
    .ok();

    Ok(Json(VerifyEmailResponse {
        verified: true,
        message: "Email verified successfully".to_string(),
    }))
}

/// Resend verification email
pub async fn resend_verification(
    State(state): State<Arc<AppState>>,
    RequireAuth(auth_user): RequireAuth,
) -> ApiResult<StatusCode> {
    let user_repo = UserRepository::new(&state.db);
    let verification_repo = EmailVerificationRepository::new(&state.db);

    // Get current user (auth_user.id is the user ID for session auth)
    let user_id = Uuid::parse_str(&auth_user.id)
        .map_err(|_| ApiError::BadRequest("Invalid user ID".to_string()))?;
    let user = user_repo
        .find_by_id(user_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("User not found".to_string()))?;

    // Check if already verified
    if user.email_verified {
        return Err(ApiError::BadRequest("Email already verified".to_string()));
    }

    // Delete any existing verification tokens for this user
    verification_repo.delete_for_user(user.id).await?;

    // Create new verification token (24 hours validity)
    let (token, _raw_token) = EmailVerificationToken::new(user.id, 24);
    verification_repo.create(&token).await?;

    // In production, you would send an email here with the token
    // For now, we just create the token and return success
    // The raw_token would be included in the verification link sent via email

    Ok(StatusCode::NO_CONTENT)
}
