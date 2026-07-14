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
    /// Approve a promotion/rollback change request under dual-control. Kept
    /// **separate** from `bundle:promote` so separation-of-duties can grant the
    /// authority to *request* a promotion and the authority to *approve* one to
    /// different principals (e.g. a deploy pipeline holds `bundle:promote`; a
    /// change-approval board holds `bundle:approve`).
    #[serde(rename = "bundle:approve")]
    BundleApprove,

    // Deployment permissions (propagation surface: rollout / rollback /
    // approve-wave / cancel / pin). Held by deploy pipelines and operators;
    // deliberately NOT implied by mere org membership — a read-only service
    // token must never be able to roll the fleet (SEC R2-1).
    #[serde(rename = "deployment:write")]
    DeploymentWrite,

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

    /// Erase a data subject's personal data (GDPR Art. 17 / DPA-2018 DSAR).
    /// Kept **separate** from `org:admin` under separation-of-duties: erasure is
    /// irreversible and destroys evidence, so the authority to *run* an erasure
    /// can be granted to a dedicated privacy/DPO role distinct from the operators
    /// who hold general org administration. (`admin` still covers it.)
    #[serde(rename = "audit:erase")]
    AuditErase,

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
            Self::BundleApprove => "bundle:approve",
            Self::DeploymentWrite => "deployment:write",
            Self::OrgRead => "org:read",
            Self::OrgWrite => "org:write",
            Self::OrgAdmin => "org:admin",
            Self::ApiKeyRead => "apikey:read",
            Self::ApiKeyWrite => "apikey:write",
            Self::AuditErase => "audit:erase",
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
            "bundle:approve" => Some(Self::BundleApprove),
            "deployment:write" => Some(Self::DeploymentWrite),
            "org:read" => Some(Self::OrgRead),
            "org:write" => Some(Self::OrgWrite),
            "org:admin" => Some(Self::OrgAdmin),
            "apikey:read" => Some(Self::ApiKeyRead),
            "apikey:write" => Some(Self::ApiKeyWrite),
            "audit:erase" => Some(Self::AuditErase),
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
            Self::BundleApprove,
            Self::DeploymentWrite,
            Self::OrgRead,
            Self::OrgWrite,
            Self::OrgAdmin,
            Self::ApiKeyRead,
            Self::ApiKeyWrite,
            Self::AuditErase,
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

    #[test]
    fn test_bundle_approve_is_separate_from_promote() {
        // Round-trips and, crucially for separation of duties, is a distinct
        // authority: holding one does not confer the other.
        assert_eq!(Scope::parse("bundle:approve"), Some(Scope::BundleApprove));
        assert_eq!(Scope::BundleApprove.as_str(), "bundle:approve");

        let promoter = Permission::from_scopes(vec![Scope::BundlePromote]);
        assert!(!promoter.has(Scope::BundleApprove));

        let approver = Permission::from_scopes(vec![Scope::BundleApprove]);
        assert!(!approver.has(Scope::BundlePromote));

        // A genuine platform admin still covers approval.
        assert!(Permission::from_scopes(vec![Scope::Admin]).has(Scope::BundleApprove));
    }

    #[test]
    fn test_deployment_write_is_a_distinct_deploy_authority() {
        // SEC R2-1: the propagation surface needs its own scope. It round-trips
        // and is NOT conferred by read scopes or bundle write.
        assert_eq!(
            Scope::parse("deployment:write"),
            Some(Scope::DeploymentWrite)
        );
        assert_eq!(Scope::DeploymentWrite.as_str(), "deployment:write");

        let read_only =
            Permission::from_scopes(vec![Scope::AgentRead, Scope::PolicyRead, Scope::BundleRead]);
        assert!(!read_only.has(Scope::DeploymentWrite));
        assert!(!Permission::from_scopes(vec![Scope::BundleWrite]).has(Scope::DeploymentWrite));

        // Platform admin still covers it.
        assert!(Permission::from_scopes(vec![Scope::Admin]).has(Scope::DeploymentWrite));
    }

    #[test]
    fn test_audit_erase_is_separate_from_org_admin() {
        // Separation of duties: erasure is irreversible + destroys evidence, so
        // it is a distinct authority. It round-trips and is NOT conferred by
        // org:admin (a general operator must not be able to erase a subject).
        assert_eq!(Scope::parse("audit:erase"), Some(Scope::AuditErase));
        assert_eq!(Scope::AuditErase.as_str(), "audit:erase");

        let org_admin = Permission::from_scopes(vec![Scope::OrgAdmin]);
        assert!(!org_admin.has(Scope::AuditErase));

        let eraser = Permission::from_scopes(vec![Scope::AuditErase]);
        assert!(!eraser.has(Scope::OrgAdmin));

        // The global platform admin still covers erasure.
        assert!(Permission::from_scopes(vec![Scope::Admin]).has(Scope::AuditErase));
    }

    #[test]
    fn test_all_scopes_round_trip() {
        for scope in Scope::all() {
            assert_eq!(Scope::parse(scope.as_str()), Some(scope), "{scope}");
        }
    }
}
