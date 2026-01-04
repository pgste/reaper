//! Authentication module for Reaper Management Server
//!
//! Provides:
//! - API key authentication for initial agent registration
//! - JWT token generation and validation
//! - JWKS endpoint support for external identity providers
//! - Permission scopes for fine-grained access control

pub mod api_key;
pub mod jwt;
pub mod middleware;
pub mod scopes;

pub use api_key::{ApiKey, ApiKeyGenerator, ApiKeyRepository};
pub use jwt::{Claims, JwtManager};
pub use middleware::{AuthenticatedUser, RequireAuth};
pub use scopes::{Permission, Scope};
