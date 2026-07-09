//! Enterprise SSO — per-org identity-provider configuration and the session
//! broker that turns an external assertion into a Reaper session.
//!
//! Plan 03, Phase 1 ships native OIDC; SAML and SCIM layer on later through the
//! same [`broker`] seam.

pub mod store;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// SSO protocol. Only OIDC is wired today; SAML is a later phase through the
/// same broker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SsoProtocol {
    Oidc,
}

impl SsoProtocol {
    pub fn as_str(self) -> &'static str {
        match self {
            SsoProtocol::Oidc => "oidc",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "oidc" => Some(SsoProtocol::Oidc),
            _ => None,
        }
    }
}

/// A per-org SSO identity-provider configuration.
///
/// The client secret is stored **encrypted** (`client_secret_encrypted`) and
/// never serialized out — an admin can update it but can't read it back.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SsoConfig {
    pub id: Uuid,
    pub org_id: Uuid,
    pub protocol: SsoProtocol,
    pub enabled: bool,
    /// IdP issuer — the `iss` claim value and the OIDC discovery base.
    pub issuer: String,
    pub client_id: String,
    /// Encrypted client secret (never serialized). `None` for public/PKCE-only
    /// clients.
    #[serde(skip_serializing)]
    pub client_secret_encrypted: Option<String>,
    /// Explicit discovery document URL; derived from `issuer` when absent.
    pub discovery_url: Option<String>,
    /// Explicit JWKS URL override; taken from discovery when absent.
    pub jwks_url: Option<String>,
    /// Raw attribute-map JSON (groups claim + group→role map + claim overrides).
    /// Parsed lazily by the broker so a malformed map can't break config CRUD.
    pub attr_map_json: Option<String>,
    /// Raw allowed-domains JSON (`["example.com"]`); empty/absent = any verified
    /// email the IdP asserts.
    pub allowed_domains_json: Option<String>,
    /// Role assigned to a user whose groups don't match the attribute map.
    pub default_role: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl SsoConfig {
    /// The effective discovery URL: explicit override, else derived from the
    /// issuer per the OIDC discovery spec.
    pub fn effective_discovery_url(&self) -> String {
        if let Some(url) = &self.discovery_url {
            return url.clone();
        }
        format!(
            "{}/.well-known/openid-configuration",
            self.issuer.trim_end_matches('/')
        )
    }

    /// Parsed allowed email domains (lowercased). Empty = no restriction.
    pub fn allowed_domains(&self) -> Vec<String> {
        self.allowed_domains_json
            .as_deref()
            .and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
            .unwrap_or_default()
            .into_iter()
            .map(|d| d.trim().to_ascii_lowercase())
            .filter(|d| !d.is_empty())
            .collect()
    }
}

/// Input to create/update a per-org SSO config. The secret is provided in the
/// clear here and encrypted by the handler before it reaches the store.
#[derive(Debug, Clone)]
pub struct SsoConfigInput {
    pub protocol: SsoProtocol,
    pub enabled: bool,
    pub issuer: String,
    pub client_id: String,
    pub client_secret_encrypted: Option<String>,
    pub discovery_url: Option<String>,
    pub jwks_url: Option<String>,
    pub attr_map_json: Option<String>,
    pub allowed_domains_json: Option<String>,
    pub default_role: String,
}
