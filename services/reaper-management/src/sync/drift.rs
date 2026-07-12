//! Drift detection between a git source's HEAD and its deployed policies
//! (Plan 09 Step 8).
//!
//! "Drift" is when the policies the control plane has deployed for a source no
//! longer match what the source's git repo says they should be — e.g. someone
//! edited a policy directly through the API (out-of-band of git), or a commit
//! landed in the repo that hasn't been synced yet. Surfacing it as a
//! first-class, queryable state is what makes "git is the source of truth"
//! enforceable rather than aspirational.
//!
//! The diff itself is a pure set comparison over `name -> content` maps, so it
//! is trivially testable; the handler supplies the two sides (git working tree
//! vs deployed policy rows).

use serde::Serialize;
use std::collections::BTreeMap;
use utoipa::ToSchema;

/// Derive the org-unique policy name for a synced file. Namespaced by the
/// source name so two sources shipping the same path never collide, with the
/// extension stripped and separators flattened: source "prod-policies" +
/// "auth/rbac.reap" → "prod-policies/auth-rbac". Shared by materialization
/// (which writes these names) and drift detection (which compares them).
pub fn source_policy_name(source_name: &str, file_path: &str) -> String {
    let no_ext = std::path::Path::new(file_path)
        .with_extension("")
        .to_string_lossy()
        .replace(['/', '\\'], "-");
    format!("{source_name}/{no_ext}")
}

/// Overall drift status for a source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DriftStatus {
    InSync,
    Drift,
}

/// The computed difference between git HEAD and the deployed policy set.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DriftReport {
    pub status: DriftStatus,
    /// Policy names present at git HEAD but not deployed.
    pub added: Vec<String>,
    /// Policy names deployed but absent at git HEAD.
    pub removed: Vec<String>,
    /// Policy names present in both but with differing content.
    pub changed: Vec<String>,
}

/// Compute drift between the policy set at git HEAD and the deployed set, each
/// as a `name -> content` map. Deterministic and order-independent (`BTreeMap`
/// keeps the output name lists sorted).
pub fn compute_drift(
    git_head: &BTreeMap<String, String>,
    deployed: &BTreeMap<String, String>,
) -> DriftReport {
    let mut added = Vec::new();
    let mut changed = Vec::new();
    for (name, head_content) in git_head {
        match deployed.get(name) {
            None => added.push(name.clone()),
            Some(deployed_content) if deployed_content != head_content => {
                changed.push(name.clone())
            }
            Some(_) => {}
        }
    }
    let removed: Vec<String> = deployed
        .keys()
        .filter(|name| !git_head.contains_key(*name))
        .cloned()
        .collect();

    let status = if added.is_empty() && removed.is_empty() && changed.is_empty() {
        DriftStatus::InSync
    } else {
        DriftStatus::Drift
    };

    DriftReport {
        status,
        added,
        removed,
        changed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn identical_sets_are_in_sync() {
        let a = map(&[("p1", "content1"), ("p2", "content2")]);
        let report = compute_drift(&a, &a);
        assert_eq!(report.status, DriftStatus::InSync);
        assert!(report.added.is_empty() && report.removed.is_empty() && report.changed.is_empty());
    }

    #[test]
    fn detects_added_removed_and_changed() {
        let git = map(&[("keep", "same"), ("edited", "new"), ("only_git", "x")]);
        let deployed = map(&[("keep", "same"), ("edited", "old"), ("only_deployed", "y")]);
        let report = compute_drift(&git, &deployed);
        assert_eq!(report.status, DriftStatus::Drift);
        assert_eq!(report.added, vec!["only_git"]);
        assert_eq!(report.removed, vec!["only_deployed"]);
        assert_eq!(report.changed, vec!["edited"]);
    }

    #[test]
    fn out_of_band_edit_shows_as_changed() {
        // The acceptance case: a policy deployed via the API diverges from git.
        let git = map(&[("rbac", "allow admin")]);
        let deployed = map(&[("rbac", "allow admin\nallow everyone")]);
        let report = compute_drift(&git, &deployed);
        assert_eq!(report.status, DriftStatus::Drift);
        assert_eq!(report.changed, vec!["rbac"]);
    }

    #[test]
    fn empty_both_is_in_sync() {
        let empty = BTreeMap::new();
        assert_eq!(compute_drift(&empty, &empty).status, DriftStatus::InSync);
    }

    #[test]
    fn output_lists_are_sorted() {
        let git = map(&[("b", "1"), ("a", "1"), ("c", "1")]);
        let deployed = BTreeMap::new();
        let report = compute_drift(&git, &deployed);
        assert_eq!(report.added, vec!["a", "b", "c"]);
    }
}
