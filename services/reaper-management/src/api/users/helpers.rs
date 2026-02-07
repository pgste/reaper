//! Helper functions for user authentication and validation.

use axum::http::header::HeaderMap;

use crate::{
    api::error::{ApiError, ApiResult},
    auth::users::{SessionRepository, User, UserError, UserRepository},
    state::AppState,
};

/// Get user from session token in headers
pub async fn get_user_from_session(state: &AppState, headers: &HeaderMap) -> ApiResult<User> {
    let token = get_session_token(headers)?;

    let session_repo = SessionRepository::new(&state.db);
    let session = session_repo
        .find_by_token(&token)
        .await
        .map_err(|e| match e {
            UserError::SessionExpired => ApiError::Unauthorized("Session expired".to_string()),
            _ => ApiError::Internal(format!("Session lookup failed: {}", e)),
        })?
        .ok_or_else(|| ApiError::Unauthorized("Invalid session".to_string()))?;

    let user_repo = UserRepository::new(&state.db);
    user_repo
        .find_by_id(session.user_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("User not found".to_string()))
}

/// Extract session token from headers
pub fn get_session_token(headers: &HeaderMap) -> ApiResult<String> {
    let auth_header = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .ok_or_else(|| {
            ApiError::Unauthorized("Missing or invalid Authorization header".to_string())
        })?;

    if !auth_header.starts_with("rst_") {
        return Err(ApiError::Unauthorized(
            "Invalid session token format".to_string(),
        ));
    }

    Ok(auth_header.to_string())
}

/// Validate email format
pub fn is_valid_email(email: &str) -> bool {
    // Basic email validation
    let parts: Vec<&str> = email.split('@').collect();
    if parts.len() != 2 {
        return false;
    }
    let (local, domain) = (parts[0], parts[1]);

    !local.is_empty()
        && !domain.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
}

/// Validate password strength
pub fn validate_password(password: &str) -> Result<(), String> {
    if password.len() < 8 {
        return Err("Password must be at least 8 characters long".to_string());
    }
    if password.len() > 128 {
        return Err("Password must be at most 128 characters long".to_string());
    }
    // Additional checks can be added here (uppercase, numbers, special chars)
    Ok(())
}

/// Create a URL-friendly slug from a string
pub fn slugify(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<&str>>()
        .join("-")
}

/// Validate org slug format
pub fn is_valid_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug.len() <= 50
        && slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !slug.starts_with('-')
        && !slug.ends_with('-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_email_validation() {
        assert!(is_valid_email("test@example.com"));
        assert!(is_valid_email("user.name@domain.co.uk"));
        assert!(!is_valid_email("invalid"));
        assert!(!is_valid_email("@example.com"));
        assert!(!is_valid_email("test@"));
        assert!(!is_valid_email("test@.com"));
    }

    #[test]
    fn test_password_validation() {
        assert!(validate_password("SecurePass123!").is_ok());
        assert!(validate_password("short").is_err());
        assert!(validate_password("12345678").is_ok());
    }

    #[test]
    fn test_slugify() {
        assert_eq!(slugify("Acme Corp"), "acme-corp");
        assert_eq!(slugify("My Company!"), "my-company");
        assert_eq!(slugify("hello---world"), "hello-world");
    }

    #[test]
    fn test_slug_validation() {
        assert!(is_valid_slug("acme-corp"));
        assert!(is_valid_slug("company123"));
        assert!(!is_valid_slug("-invalid"));
        assert!(!is_valid_slug("invalid-"));
        assert!(!is_valid_slug("UPPERCASE"));
    }
}
