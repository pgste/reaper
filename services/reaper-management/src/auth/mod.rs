//! Authentication module for Reaper Management Server
//!
//! Provides:
//! - User authentication with email/password
//! - Session-based authentication for web users
//! - API key authentication for initial agent registration
//! - JWT token generation and validation
//! - JWKS endpoint support for external identity providers
//! - Permission scopes for fine-grained access control
//! - mTLS client certificate validation

pub mod api_key;
pub mod jwks;
pub mod jwt;
pub mod middleware;
pub mod mtls;
pub mod scopes;
pub mod users;

pub use api_key::{ApiKey, ApiKeyGenerator, ApiKeyRepository};
pub use jwks::{JwksClaims, JwksConfig, JwksConfigRepository, JwksError, JwksValidator};
pub use jwt::{Claims, JwtManager};
pub use middleware::{AuthenticatedUser, RequireAuth};
pub use mtls::{ClientCertificate, ClientCertificateRepository, MtlsError, RegisterCertificate};
pub use scopes::{Permission, Scope};
pub use users::{
    hash_token, OrgRole, PasswordResetRepository, PasswordResetToken, Session, SessionRepository,
    User, UserError, UserOrg, UserOrgRepository, UserRepository, UserStatus,
};
