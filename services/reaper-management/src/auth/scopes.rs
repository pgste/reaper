//! Permission scopes for access control
//!
//! Defines the permission model for API access.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Permission scope
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    // Agent permissions
    #[serde(rename = "agent:register")]
    AgentRegister,
    #[serde(rename = "agent:read")]
    AgentRead,
    #[serde(rename = "agent:write")]
    AgentWrite,

    // Policy permissions
    #[serde(rename = "policy:read")]
    PolicyRead,
    #[serde(rename = "policy:write")]
    PolicyWrite,

    // Bundle permissions
    #[serde(rename = "bundle:read")]
    BundleRead,
    #[serde(rename = "bundle:write")]
    BundleWrite,
    #[serde(rename = "bundle:promote")]
    BundlePromote,

    // Organization permissions
    #[serde(rename = "org:read")]
    OrgRead,
    #[serde(rename = "org:write")]
    OrgWrite,
    #[serde(rename = "org:admin")]
    OrgAdmin,

    // API key management
    #[serde(rename = "apikey:read")]
    ApiKeyRead,
    #[serde(rename = "apikey:write")]
    ApiKeyWrite,

    // Full admin access
    #[serde(rename = "admin")]
    Admin,
}

impl Scope {
    /// Get the string representation of the scope
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AgentRegister => "agent:register",
            Self::AgentRead => "agent:read",
            Self::AgentWrite => "agent:write",
            Self::PolicyRead => "policy:read",
            Self::PolicyWrite => "policy:write",
            Self::BundleRead => "bundle:read",
            Self::BundleWrite => "bundle:write",
            Self::BundlePromote => "bundle:promote",
            Self::OrgRead => "org:read",
            Self::OrgWrite => "org:write",
            Self::OrgAdmin => "org:admin",
            Self::ApiKeyRead => "apikey:read",
            Self::ApiKeyWrite => "apikey:write",
            Self::Admin => "admin",
        }
    }

    /// Parse a scope from string
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "agent:register" => Some(Self::AgentRegister),
            "agent:read" => Some(Self::AgentRead),
            "agent:write" => Some(Self::AgentWrite),
            "policy:read" => Some(Self::PolicyRead),
            "policy:write" => Some(Self::PolicyWrite),
            "bundle:read" => Some(Self::BundleRead),
            "bundle:write" => Some(Self::BundleWrite),
            "bundle:promote" => Some(Self::BundlePromote),
            "org:read" => Some(Self::OrgRead),
            "org:write" => Some(Self::OrgWrite),
            "org:admin" => Some(Self::OrgAdmin),
            "apikey:read" => Some(Self::ApiKeyRead),
            "apikey:write" => Some(Self::ApiKeyWrite),
            "admin" => Some(Self::Admin),
            _ => None,
        }
    }

    /// Get all scopes
    pub fn all() -> Vec<Self> {
        vec![
            Self::AgentRegister,
            Self::AgentRead,
            Self::AgentWrite,
            Self::PolicyRead,
            Self::PolicyWrite,
            Self::BundleRead,
            Self::BundleWrite,
            Self::BundlePromote,
            Self::OrgRead,
            Self::OrgWrite,
            Self::OrgAdmin,
            Self::ApiKeyRead,
            Self::ApiKeyWrite,
            Self::Admin,
        ]
    }

    /// Get default scopes for agent API keys
    pub fn agent_defaults() -> Vec<Self> {
        vec![Self::AgentRegister, Self::AgentRead]
    }

    /// Get admin scopes
    pub fn admin_scopes() -> Vec<Self> {
        Self::all()
    }
}

impl std::fmt::Display for Scope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Permission checker
#[derive(Debug, Clone, Default)]
pub struct Permission {
    scopes: HashSet<Scope>,
}

impl Permission {
    /// Create new permission set
    pub fn new() -> Self {
        Self {
            scopes: HashSet::new(),
        }
    }

    /// Create from scope list
    pub fn from_scopes(scopes: Vec<Scope>) -> Self {
        Self {
            scopes: scopes.into_iter().collect(),
        }
    }

    /// Create from string list
    pub fn from_strings(scope_strs: &[String]) -> Self {
        let scopes = scope_strs.iter().filter_map(|s| Scope::parse(s)).collect();
        Self { scopes }
    }

    /// Add a scope
    pub fn add(&mut self, scope: Scope) {
        self.scopes.insert(scope);
    }

    /// Check if permission is granted
    pub fn has(&self, scope: Scope) -> bool {
        self.scopes.contains(&Scope::Admin) || self.scopes.contains(&scope)
    }

    /// Check if any of the required scopes is granted
    pub fn has_any(&self, scopes: &[Scope]) -> bool {
        if self.scopes.contains(&Scope::Admin) {
            return true;
        }
        scopes.iter().any(|s| self.scopes.contains(s))
    }

    /// Check if all required scopes are granted
    pub fn has_all(&self, scopes: &[Scope]) -> bool {
        if self.scopes.contains(&Scope::Admin) {
            return true;
        }
        scopes.iter().all(|s| self.scopes.contains(s))
    }

    /// Get all scopes as strings
    pub fn to_strings(&self) -> Vec<String> {
        self.scopes.iter().map(|s| s.to_string()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scope_parsing() {
        assert_eq!(Scope::parse("agent:register"), Some(Scope::AgentRegister));
        assert_eq!(Scope::parse("admin"), Some(Scope::Admin));
        assert_eq!(Scope::parse("invalid"), None);
    }

    #[test]
    fn test_permission_has() {
        let mut perm = Permission::new();
        perm.add(Scope::AgentRead);
        perm.add(Scope::PolicyRead);

        assert!(perm.has(Scope::AgentRead));
        assert!(perm.has(Scope::PolicyRead));
        assert!(!perm.has(Scope::PolicyWrite));
    }

    #[test]
    fn test_admin_has_all() {
        let perm = Permission::from_scopes(vec![Scope::Admin]);

        assert!(perm.has(Scope::AgentRead));
        assert!(perm.has(Scope::PolicyWrite));
        assert!(perm.has(Scope::OrgAdmin));
    }

    #[test]
    fn test_has_any() {
        let perm = Permission::from_scopes(vec![Scope::AgentRead]);

        assert!(perm.has_any(&[Scope::AgentRead, Scope::PolicyRead]));
        assert!(!perm.has_any(&[Scope::PolicyRead, Scope::PolicyWrite]));
    }
}
