//! SCIM 2.0 provisioning support (Plan 03, Phase 2).
//!
//! A per-org bearer token authenticates an IdP's directory-sync client to the
//! `/scim/v2/*` endpoints; [`store`] persists those tokens (hashed at rest). The
//! actual user CRUD lives in `api/scim` and reuses the same user/session
//! repositories as the OIDC broker.

pub mod store;

use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

/// A per-org SCIM provisioning token (metadata; the secret lives only as a hash).
#[derive(Debug, Clone, Serialize)]
pub struct ScimToken {
    pub id: Uuid,
    pub org_id: Uuid,
    pub name: String,
    #[serde(skip_serializing)]
    pub token_hash: String,
    pub created_by: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub revoked: bool,
}
