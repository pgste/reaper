//! Policy Compiler - Converts Reaper policies to eBPF format
//!
//! This module compiles high-level policy rules into the low-level format
//! required by eBPF maps. It handles resource path conversion, action mapping,
//! and UID/GID extraction.

use crate::types::{PolicyAction, PolicyEntry, MAX_PATH_LEN};
use anyhow::Result;
use policy_engine::{PolicyRule, SimplePolicyEvaluator};
use tracing::debug;

/// Compiles Reaper policies into eBPF-compatible format
pub struct PolicyCompiler {
    /// Default UID for policies (None = no UID check)
    default_uid: Option<u32>,

    /// Default GID for policies (None = no GID check)
    default_gid: Option<u32>,
}

impl PolicyCompiler {
    /// Create a new compiler
    pub fn new() -> Self {
        Self {
            default_uid: None,
            default_gid: None,
        }
    }

    /// Set default UID for policies
    pub fn with_default_uid(mut self, uid: u32) -> Self {
        self.default_uid = Some(uid);
        self
    }

    /// Set default GID for policies
    pub fn with_default_gid(mut self, gid: u32) -> Self {
        self.default_gid = Some(gid);
        self
    }

    /// Compile a Simple policy rule to eBPF format
    ///
    /// # Arguments
    /// * `rule` - The policy rule to compile
    /// * `priority` - Rule priority (lower = higher priority)
    ///
    /// # Returns
    /// Tuple of (resource_key, policy_entry)
    pub fn compile_rule(
        &self,
        rule: &PolicyRule,
        priority: u32,
    ) -> Result<([u8; MAX_PATH_LEN], PolicyEntry)> {
        // Convert resource to fixed-size key
        let key = self.resource_to_key(&rule.resource)?;

        // Convert action
        let action = match &rule.action {
            policy_engine::PolicyAction::Allow => PolicyAction::Allow,
            policy_engine::PolicyAction::Deny => PolicyAction::Deny,
            policy_engine::PolicyAction::Log => PolicyAction::Log,
        };

        // Create policy entry
        let mut entry = PolicyEntry::new(action).with_priority(priority);

        // Add UID/GID checks if specified
        if let Some(uid) = self.default_uid {
            entry = entry.with_uid(uid);
        }

        if let Some(gid) = self.default_gid {
            entry = entry.with_gid(gid);
        }

        // TODO: Parse conditions for UID/GID requirements
        // For now, conditions are not compiled to eBPF

        debug!(
            "Compiled rule: {} → {:?} (priority: {})",
            rule.resource, action, priority
        );

        Ok((key, entry))
    }

    /// Compile all rules from a Simple policy evaluator
    ///
    /// Returns a vector of (resource_key, policy_entry) tuples
    pub fn compile_simple_policy(
        &self,
        evaluator: &SimplePolicyEvaluator,
    ) -> Result<Vec<([u8; MAX_PATH_LEN], PolicyEntry)>> {
        let mut compiled = Vec::new();

        for (index, rule) in evaluator.rules.iter().enumerate() {
            let (key, entry) = self.compile_rule(rule, index as u32)?;
            compiled.push((key, entry));
        }

        debug!("Compiled {} rules from Simple policy", compiled.len());

        Ok(compiled)
    }

    /// Convert a resource path to a fixed-size BPF map key
    ///
    /// Handles:
    /// - Exact paths: "/api/users" → ["/api/users\0", ...]
    /// - Wildcards: "*" → special encoding
    /// - Path truncation if too long
    pub fn resource_to_key(&self, resource: &str) -> Result<[u8; MAX_PATH_LEN]> {
        let mut key = [0u8; MAX_PATH_LEN];

        // Handle wildcard specially (first byte = 0xFF marker)
        if resource == "*" {
            key[0] = 0xFF; // Wildcard marker
            return Ok(key);
        }

        // Copy resource bytes to key (truncate if necessary)
        let bytes = resource.as_bytes();
        let len = bytes.len().min(MAX_PATH_LEN - 1); // Reserve 1 byte for null terminator

        if bytes.len() > MAX_PATH_LEN - 1 {
            tracing::warn!(
                "Resource path truncated: {} bytes → {} bytes",
                bytes.len(),
                len
            );
        }

        key[..len].copy_from_slice(&bytes[..len]);
        // key[len] = 0; // Null terminator (already 0)

        Ok(key)
    }

    /// Convert a key back to a resource string (for debugging)
    pub fn key_to_resource(&self, key: &[u8; MAX_PATH_LEN]) -> String {
        // Check for wildcard
        if key[0] == 0xFF {
            return "*".to_string();
        }

        // Find null terminator
        let len = key.iter().position(|&b| b == 0).unwrap_or(MAX_PATH_LEN);

        String::from_utf8_lossy(&key[..len]).to_string()
    }

    /// Create a policy entry for a specific decision
    ///
    /// This is used by the learning engine to create entries for
    /// frequently accessed paths.
    pub fn compile_decision(
        &self,
        resource: &str,
        action: policy_engine::PolicyAction,
        uid: Option<u32>,
        gid: Option<u32>,
        priority: u32,
    ) -> Result<([u8; MAX_PATH_LEN], PolicyEntry)> {
        let key = self.resource_to_key(resource)?;

        let ebpf_action = match action {
            policy_engine::PolicyAction::Allow => PolicyAction::Allow,
            policy_engine::PolicyAction::Deny => PolicyAction::Deny,
            policy_engine::PolicyAction::Log => PolicyAction::Log,
        };

        let mut entry = PolicyEntry::new(ebpf_action).with_priority(priority);

        if let Some(uid) = uid.or(self.default_uid) {
            entry = entry.with_uid(uid);
        }

        if let Some(gid) = gid.or(self.default_gid) {
            entry = entry.with_gid(gid);
        }

        Ok((key, entry))
    }
}

impl Default for PolicyCompiler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resource_to_key() {
        let compiler = PolicyCompiler::new();

        // Test exact path
        let key = compiler.resource_to_key("/api/users").unwrap();
        assert_eq!(&key[0..10], b"/api/users");
        assert_eq!(key[10], 0); // Null terminator

        // Test wildcard
        let key = compiler.resource_to_key("*").unwrap();
        assert_eq!(key[0], 0xFF);

        // Test round-trip
        let resource = "/api/users/123";
        let key = compiler.resource_to_key(resource).unwrap();
        let recovered = compiler.key_to_resource(&key);
        assert_eq!(resource, recovered);
    }

    #[test]
    fn test_compile_rule() {
        let compiler = PolicyCompiler::new();

        let rule = PolicyRule {
            action: policy_engine::PolicyAction::Allow,
            resource: "/api/users".to_string(),
            conditions: vec![],
        };

        let (key, entry) = compiler.compile_rule(&rule, 0).unwrap();

        assert_eq!(entry.action, PolicyAction::Allow as u8);
        assert_eq!(entry.priority, 0);
        assert_eq!(entry.flags, 0); // No UID/GID checks

        let resource = compiler.key_to_resource(&key);
        assert_eq!(resource, "/api/users");
    }

    #[test]
    fn test_compile_with_uid_gid() {
        let compiler = PolicyCompiler::new()
            .with_default_uid(1000)
            .with_default_gid(1000);

        let rule = PolicyRule {
            action: policy_engine::PolicyAction::Deny,
            resource: "/etc/passwd".to_string(),
            conditions: vec![],
        };

        let (_key, entry) = compiler.compile_rule(&rule, 0).unwrap();

        assert_eq!(entry.action, PolicyAction::Deny as u8);
        assert_eq!(entry.flags, 0x03); // UID and GID checks enabled
        assert_eq!(entry.required_uid, 1000);
        assert_eq!(entry.required_gid, 1000);
    }

    #[test]
    fn test_compile_simple_policy() {
        let compiler = PolicyCompiler::new();

        let rules = vec![
            PolicyRule {
                action: policy_engine::PolicyAction::Allow,
                resource: "/api/public".to_string(),
                conditions: vec![],
            },
            PolicyRule {
                action: policy_engine::PolicyAction::Deny,
                resource: "/api/admin".to_string(),
                conditions: vec![],
            },
        ];

        let evaluator = SimplePolicyEvaluator::new(rules);
        let compiled = compiler.compile_simple_policy(&evaluator).unwrap();

        assert_eq!(compiled.len(), 2);

        // Check first rule
        assert_eq!(compiled[0].1.action, PolicyAction::Allow as u8);
        assert_eq!(compiled[0].1.priority, 0);

        // Check second rule
        assert_eq!(compiled[1].1.action, PolicyAction::Deny as u8);
        assert_eq!(compiled[1].1.priority, 1);
    }
}
