//! Policy mapping registry — maps Reaper policy names to OPA equivalents
//!
//! The 12 equivalent `.rego` policies live at `benchmarks/reaper-vs-opa/policies/opa/`.
//! This module provides the mapping so the comparison benchmark knows which OPA
//! package/rule to call for each Reaper policy.

/// Mapping between a Reaper policy and its OPA equivalent.
#[derive(Debug, Clone)]
pub struct PolicyMapping {
    /// Reaper policy name as deployed in the agent
    pub reaper_policy_name: &'static str,
    /// OPA .rego filename (without path), e.g. "rbac.rego"
    pub opa_rego_file: &'static str,
    /// OPA package path (dots replaced with slashes for the data API), e.g. "reaper/rbac"
    pub opa_package_path: &'static str,
    /// OPA rule name, e.g. "allow"
    pub opa_rule: &'static str,
}

/// All 12 policy mappings between Reaper and OPA.
pub const POLICY_MAPPINGS: &[PolicyMapping] = &[
    PolicyMapping {
        reaper_policy_name: "rbac_simple",
        opa_rego_file: "rbac.rego",
        opa_package_path: "reaper/rbac",
        opa_rule: "allow",
    },
    PolicyMapping {
        reaper_policy_name: "abac_clearance",
        opa_rego_file: "abac.rego",
        opa_package_path: "reaper/abac",
        opa_rule: "allow",
    },
    PolicyMapping {
        reaper_policy_name: "rebac_relationships",
        opa_rego_file: "rebac.rego",
        opa_package_path: "reaper/rebac",
        opa_rule: "allow",
    },
    PolicyMapping {
        reaper_policy_name: "multilayer_enterprise",
        opa_rego_file: "multilayer.rego",
        opa_package_path: "reaper/multilayer",
        opa_rule: "allow",
    },
    PolicyMapping {
        reaper_policy_name: "math_validation",
        opa_rego_file: "math.rego",
        opa_package_path: "reaper/math",
        opa_rule: "allow",
    },
    PolicyMapping {
        reaper_policy_name: "regex_validation",
        opa_rego_file: "regex.rego",
        opa_package_path: "reaper/regex",
        opa_rule: "allow",
    },
    PolicyMapping {
        reaper_policy_name: "string_operations",
        opa_rego_file: "string.rego",
        opa_package_path: "reaper/string",
        opa_rule: "allow",
    },
    PolicyMapping {
        reaper_policy_name: "collection_operations",
        opa_rego_file: "collection.rego",
        opa_package_path: "reaper/collection",
        opa_rule: "allow",
    },
    PolicyMapping {
        reaper_policy_name: "time_based_access",
        opa_rego_file: "time.rego",
        opa_package_path: "reaper/time",
        opa_rule: "allow",
    },
    PolicyMapping {
        reaper_policy_name: "comprehensions",
        opa_rego_file: "comprehension.rego",
        opa_package_path: "reaper/comprehension",
        opa_rule: "allow",
    },
    PolicyMapping {
        reaper_policy_name: "json_operations",
        opa_rego_file: "json.rego",
        opa_package_path: "reaper/json",
        opa_rule: "allow",
    },
    PolicyMapping {
        reaper_policy_name: "mega_policy",
        opa_rego_file: "mega.rego",
        opa_package_path: "reaper/mega",
        opa_rule: "allow",
    },
];

/// Look up the OPA mapping for a given Reaper policy name.
pub fn get_mapping(reaper_policy_name: &str) -> Option<&'static PolicyMapping> {
    POLICY_MAPPINGS
        .iter()
        .find(|m| m.reaper_policy_name == reaper_policy_name)
}

/// Get all available policy names that have mappings.
pub fn available_policy_names() -> Vec<&'static str> {
    POLICY_MAPPINGS
        .iter()
        .map(|m| m.reaper_policy_name)
        .collect()
}
