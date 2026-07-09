//! Enterprise SSO — per-org identity-provider configuration and the session
//! broker that turns an external assertion into a Reaper session.
//!
//! Plan 03, Phase 1 ships native OIDC; SAML and SCIM layer on later through the
//! same [`broker`] seam.

pub mod broker;
pub mod store;

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Attribute mapping for an OIDC config: which ID-token claims carry the
/// email/name/groups, and how IdP group names map to Reaper org roles.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AttrMap {
    /// Claim holding the user's group list (default `groups`).
    #[serde(default)]
    pub groups_claim: Option<String>,
    /// Claim holding the user's email (default `email`).
    #[serde(default)]
    pub email_claim: Option<String>,
    /// Claim holding the display name (default `name`).
    #[serde(default)]
    pub name_claim: Option<String>,
    /// IdP group name → Reaper role (e.g. `{"reaper-admins":"owner"}`). Values
    /// that don't parse to an `OrgRole` are ignored.
    #[serde(default)]
    pub group_map: HashMap<String, String>,
}

impl AttrMap {
    pub fn groups_claim(&self) -> &str {
        self.groups_claim.as_deref().unwrap_or("groups")
    }
    pub fn email_claim(&self) -> &str {
        self.email_claim.as_deref().unwrap_or("email")
    }
    pub fn name_claim(&self) -> &str {
        self.name_claim.as_deref().unwrap_or("name")
    }
}

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

    /// Parsed attribute map (claim names + group→role map). Returns defaults if
    /// unset or malformed, so a bad map degrades to "default role for everyone"
    /// rather than breaking login.
    pub fn attr_map(&self) -> AttrMap {
        self.attr_map_json
            .as_deref()
            .and_then(|s| serde_json::from_str::<AttrMap>(s).ok())
            .unwrap_or_default()
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
