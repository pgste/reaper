//! Tests for the PolicyEngine.

use super::*;

#[tokio::test]
async fn test_policy_deployment_and_lookup() {
    let engine = PolicyEngine::new();

    let policy = EnhancedPolicy::new(
        "test-policy".to_string(),
        "Test policy".to_string(),
        vec![PolicyRule {
            action: PolicyAction::Allow,
            resource: "*".to_string(),
            conditions: vec![],
        }],
    );
    let policy_id = policy.id;

    // Deploy policy
    engine.deploy_policy(policy.clone()).unwrap();

    // Verify policy exists
    let retrieved = engine.get_policy(&policy_id).unwrap();
    assert_eq!(retrieved.name, "test-policy");

    // Verify lookup by name
    let by_name = engine.get_policy_by_name("test-policy").unwrap();
    assert_eq!(by_name.id, policy_id);
}

#[tokio::test]
async fn test_hot_swap() {
    let engine = PolicyEngine::new();

    let mut policy = EnhancedPolicy::new(
        "hot-swap".to_string(),
        "Hot swap test".to_string(),
        vec![PolicyRule {
            action: PolicyAction::Deny,
            resource: "*".to_string(),
            conditions: vec![],
        }],
    );
    let policy_id = policy.id;

    // Deploy initial policy
    engine.deploy_policy(policy.clone()).unwrap();

    // Update policy rules
    policy.update_rules(vec![PolicyRule {
        action: PolicyAction::Allow,
        resource: "*".to_string(),
        conditions: vec![],
    }]);

    // Hot swap
    engine.deploy_policy(policy).unwrap();

    // Verify new version
    let updated = engine.get_policy(&policy_id).unwrap();
    assert_eq!(updated.version, 2);
    match &updated.rules[0].action {
        PolicyAction::Allow => (),
        _ => panic!("Expected Allow action"),
    }
}

#[tokio::test]
async fn test_policy_evaluation() {
    let engine = PolicyEngine::new();

    let policy = EnhancedPolicy::new(
        "eval-test".to_string(),
        "Evaluation test".to_string(),
        vec![PolicyRule {
            action: PolicyAction::Allow,
            resource: "test-resource".to_string(),
            conditions: vec![],
        }],
    );
    let policy_id = policy.id;

    engine.deploy_policy(policy).unwrap();

    let request = PolicyRequest {
        resource: "test-resource".to_string(),
        action: "read".to_string(),
        context: std::collections::HashMap::new(),
    };

    let decision = engine.evaluate(&policy_id, &request).unwrap();

    match decision.decision {
        PolicyAction::Allow => (),
        _ => panic!("Expected Allow decision"),
    }

    assert!(decision.evaluation_time_ns > 0);
    assert_eq!(decision.matched_rule, Some(0));
}

#[tokio::test]
async fn test_tree_optimization() {
    // Create policy with tree optimization
    let rules = vec![
        PolicyRule {
            action: PolicyAction::Allow,
            resource: "resource1".to_string(),
            conditions: vec![],
        },
        PolicyRule {
            action: PolicyAction::Allow,
            resource: "resource2".to_string(),
            conditions: vec![],
        },
        PolicyRule {
            action: PolicyAction::Deny,
            resource: "*".to_string(),
            conditions: vec![],
        },
    ];

    let policy = EnhancedPolicy::new_with_tree_optimization(
        "tree-test".to_string(),
        "Tree optimization test".to_string(),
        rules,
    )
    .unwrap();

    // Verify metadata is set
    assert_eq!(
        policy.metadata.get("optimization"),
        Some(&"tree".to_string())
    );

    let policy_id = policy.id;
    let engine = PolicyEngine::new();
    engine.deploy_policy(policy).unwrap();

    // Test evaluation
    let request = PolicyRequest {
        resource: "resource1".to_string(),
        action: "read".to_string(),
        context: std::collections::HashMap::new(),
    };

    let decision = engine.evaluate(&policy_id, &request).unwrap();
    assert!(matches!(decision.decision, PolicyAction::Allow));
}

#[tokio::test]
async fn test_tree_optimization_scale() {
    // Generate many rules to test tree optimization performance
    let mut rules = Vec::new();
    for i in 0..100 {
        rules.push(PolicyRule {
            action: if i % 2 == 0 {
                PolicyAction::Allow
            } else {
                PolicyAction::Deny
            },
            resource: format!("resource_{}", i),
            conditions: vec![],
        });
    }

    // Create with tree optimization
    let tree_policy = EnhancedPolicy::new_with_tree_optimization(
        "tree-scale-test".to_string(),
        "Tree scale test".to_string(),
        rules.clone(),
    )
    .unwrap();

    // Create without tree optimization for comparison
    let linear_policy = EnhancedPolicy::new(
        "linear-scale-test".to_string(),
        "Linear scale test".to_string(),
        rules,
    );

    let engine = PolicyEngine::new();
    let tree_id = tree_policy.id;
    let linear_id = linear_policy.id;

    engine.deploy_policy(tree_policy).unwrap();
    engine.deploy_policy(linear_policy).unwrap();

    // Test both
    let request = PolicyRequest {
        resource: "resource_50".to_string(),
        action: "read".to_string(),
        context: std::collections::HashMap::new(),
    };

    let tree_decision = engine.evaluate(&tree_id, &request).unwrap();
    let linear_decision = engine.evaluate(&linear_id, &request).unwrap();

    // Both should give same result
    assert_eq!(tree_decision.decision, linear_decision.decision);

    // Tree should be faster (generally, though with only 100 rules the difference may be small)
    println!(
        "Tree eval: {}ns, Linear eval: {}ns",
        tree_decision.evaluation_time_ns, linear_decision.evaluation_time_ns
    );
}

#[tokio::test]
async fn test_metadata_flag_enables_tree() {
    let content = serde_json::to_string(&vec![PolicyRule {
        action: PolicyAction::Allow,
        resource: "test".to_string(),
        conditions: vec![],
    }])
    .unwrap();

    let mut policy = EnhancedPolicy::new_with_language(
        "metadata-test".to_string(),
        "Metadata test".to_string(),
        PolicyLanguage::Simple,
        content,
    )
    .unwrap();

    // Set tree optimization metadata
    policy
        .metadata
        .insert("optimization".to_string(), "tree".to_string());

    // Rebuild evaluator with tree optimization
    policy.build_evaluator().unwrap();

    // Verify evaluator has tree optimization enabled
    let evaluator = policy.get_evaluator().unwrap();
    if let Some(metadata) = evaluator.metadata() {
        assert!(
            metadata
                .extra
                .get("tree_optimized")
                .map(|v| v == "true")
                .unwrap_or(false),
            "Tree optimization should be enabled"
        );
    }
}

// ========== Hot-Reload Tests ==========

#[tokio::test]
async fn test_bundle_deployment_with_version_tracking() {
    use crate::reap::{Decision, Policy as ReapPolicy, ReapCondition, ReapRule};

    let engine = PolicyEngine::new();

    // Create a Reap policy
    let mut metadata = std::collections::HashMap::new();
    metadata.insert("version".to_string(), "1.0.0".to_string());

    let reap_policy = ReapPolicy {
        name: "test-bundle-policy".to_string(),
        metadata,
        default_decision: Decision::Deny,
        rules: vec![ReapRule {
            message: None,
            name: "allow-admins".to_string(),
            decision: Decision::Allow,
            condition: ReapCondition::True,
        }],
    };

    // Create bundle
    let bundle = crate::reap::PolicyBundle::new(reap_policy);

    // Deploy bundle with version tracking
    let version = engine.deploy_bundle(bundle.clone(), false).unwrap();

    assert_eq!(version.version, "1.0.0");
    assert_eq!(version.policy_id, version.policy_id);
    assert!(version.bundle_hash.len() == 32); // SHA-256 hash
    assert_eq!(
        version.deployed_at.elapsed().unwrap().as_secs(),
        0,
        "Deployment should be recent"
    );
}

#[tokio::test]
async fn test_bundle_version_history() {
    use crate::reap::{Decision, Policy as ReapPolicy};

    let engine = PolicyEngine::new();

    // Create and deploy first version
    let mut metadata1 = std::collections::HashMap::new();
    metadata1.insert("version".to_string(), "1.0.0".to_string());

    let policy1 = ReapPolicy {
        name: "versioned-policy".to_string(),
        metadata: metadata1,
        default_decision: Decision::Deny,
        rules: vec![],
    };

    let bundle1 = crate::reap::PolicyBundle::new(policy1);
    let version1 = engine.deploy_bundle(bundle1, false).unwrap();

    // Create and deploy second version
    let mut metadata2 = std::collections::HashMap::new();
    metadata2.insert("version".to_string(), "2.0.0".to_string());

    let policy2 = ReapPolicy {
        name: "another-policy".to_string(),
        metadata: metadata2,
        default_decision: Decision::Allow,
        rules: vec![],
    };

    let bundle2 = crate::reap::PolicyBundle::new(policy2);
    let version2 = engine.deploy_bundle(bundle2, false).unwrap();

    // Verify each has their own version history
    let policy1_uuid = uuid::Uuid::parse_str(&version1.policy_id).unwrap();
    let versions1 = engine.list_versions(&policy1_uuid);
    assert_eq!(versions1.len(), 1);
    assert_eq!(versions1[0].version, "1.0.0");

    let policy2_uuid = uuid::Uuid::parse_str(&version2.policy_id).unwrap();
    let versions2 = engine.list_versions(&policy2_uuid);
    assert_eq!(versions2.len(), 1);
    assert_eq!(versions2[0].version, "2.0.0");
}

#[tokio::test]
async fn test_bundle_rollback() {
    use crate::reap::{Decision, Policy as ReapPolicy};

    let engine = PolicyEngine::new();

    // Deploy version 1.0.0
    let mut metadata1 = std::collections::HashMap::new();
    metadata1.insert("version".to_string(), "1.0.0".to_string());

    let policy1 = ReapPolicy {
        name: "rollback-test".to_string(),
        metadata: metadata1,
        default_decision: Decision::Deny,
        rules: vec![],
    };

    let bundle1 = crate::reap::PolicyBundle::new(policy1);
    let version1 = engine.deploy_bundle(bundle1, false).unwrap();

    // Deploy version 2.0.0
    let mut metadata2 = std::collections::HashMap::new();
    metadata2.insert("version".to_string(), "2.0.0".to_string());

    let policy2 = ReapPolicy {
        name: "rollback-test".to_string(),
        metadata: metadata2,
        default_decision: Decision::Allow,
        rules: vec![],
    };

    let bundle2 = crate::reap::PolicyBundle::new(policy2);
    engine.deploy_bundle(bundle2, false).unwrap();

    // Rollback to 1.0.0
    let policy_uuid = uuid::Uuid::parse_str(&version1.policy_id).unwrap();
    let rollback_version = engine.rollback(&policy_uuid, "1.0.0").unwrap();

    assert_eq!(rollback_version.version, "1.0.0");

    // Verify the policy was rolled back
    let policy = engine.get_policy(&policy_uuid).unwrap();
    assert_eq!(policy.name, "rollback-test");
}

#[tokio::test]
async fn test_bundle_force_deployment() {
    use crate::reap::{Decision, Policy as ReapPolicy};

    let engine = PolicyEngine::new();

    // Create bundle
    let mut metadata = std::collections::HashMap::new();
    metadata.insert("version".to_string(), "1.0.0".to_string());

    let policy = ReapPolicy {
        name: "force-test".to_string(),
        metadata,
        default_decision: Decision::Deny,
        rules: vec![],
    };

    let bundle = crate::reap::PolicyBundle::new(policy);

    // Deploy first time
    engine.deploy_bundle(bundle.clone(), false).unwrap();

    // Deploy again with force=true (should succeed)
    let version = engine.deploy_bundle(bundle, true).unwrap();
    assert_eq!(version.version, "1.0.0");
}

#[tokio::test]
async fn test_bundle_hash_integrity() {
    use crate::reap::{Decision, Policy as ReapPolicy};

    let engine = PolicyEngine::new();

    // Create two identical bundles
    let mut metadata = std::collections::HashMap::new();
    metadata.insert("version".to_string(), "1.0.0".to_string());

    let policy = ReapPolicy {
        name: "hash-test".to_string(),
        metadata: metadata.clone(),
        default_decision: Decision::Deny,
        rules: vec![],
    };

    let bundle1 = crate::reap::PolicyBundle::new(policy.clone());
    let version1 = engine.deploy_bundle(bundle1, false).unwrap();

    // Create different bundle
    let mut metadata2 = std::collections::HashMap::new();
    metadata2.insert("version".to_string(), "2.0.0".to_string());

    let policy2 = ReapPolicy {
        name: "hash-test".to_string(),
        metadata: metadata2,
        default_decision: Decision::Allow,
        rules: vec![],
    };

    let bundle2 = crate::reap::PolicyBundle::new(policy2);
    let version2 = engine.deploy_bundle(bundle2, false).unwrap();

    // Hashes should be different
    assert_ne!(
        version1.bundle_hash, version2.bundle_hash,
        "Different bundles should have different hashes"
    );
}

#[tokio::test]
async fn test_get_version_metadata() {
    use crate::reap::{Decision, Policy as ReapPolicy};

    let engine = PolicyEngine::new();

    let mut metadata = std::collections::HashMap::new();
    metadata.insert("version".to_string(), "1.5.0".to_string());

    let policy = ReapPolicy {
        name: "version-metadata-test".to_string(),
        metadata,
        default_decision: Decision::Deny,
        rules: vec![],
    };

    let bundle = crate::reap::PolicyBundle::new(policy);
    let deployed_version = engine.deploy_bundle(bundle, false).unwrap();

    // Get version metadata
    let policy_uuid = uuid::Uuid::parse_str(&deployed_version.policy_id).unwrap();
    let retrieved_version = engine.get_version(&policy_uuid).unwrap();

    assert_eq!(retrieved_version.version, "1.5.0");
    assert_eq!(
        retrieved_version.bundle_hash, deployed_version.bundle_hash,
        "Hashes should match"
    );
}

// ========== Atomic Package Deployment Tests ==========

#[tokio::test]
async fn test_atomic_package_stage_and_commit() {
    use crate::data::DataStore;
    use crate::reap::{Decision, Policy as ReapPolicy, PolicyPackage, ReapCondition, ReapRule};

    let engine = PolicyEngine::new();
    let store = Arc::new(DataStore::new());

    // Create a package with multiple policies
    let policy1 = ReapPolicy {
        name: "atomic-policy-1".to_string(),
        metadata: std::collections::HashMap::new(),
        default_decision: Decision::Deny,
        rules: vec![ReapRule {
            message: None,
            name: "allow-admin".to_string(),
            decision: Decision::Allow,
            condition: ReapCondition::True,
        }],
    };

    let policy2 = ReapPolicy {
        name: "atomic-policy-2".to_string(),
        metadata: std::collections::HashMap::new(),
        default_decision: Decision::Allow,
        rules: vec![],
    };

    let package = PolicyPackage::new(
        "test-package".to_string(),
        "1.0.0".to_string(),
        vec![policy1, policy2],
    );

    // Stage the package
    let staged = engine.stage_package(&package, store.clone()).unwrap();

    assert!(staged.is_valid());
    assert_eq!(staged.staged_policy_ids.len(), 2);
    assert!(engine.is_staging_in_progress());

    // Verify policies are NOT visible yet
    assert!(engine.get_policy_by_name("atomic-policy-1").is_none());
    assert!(engine.get_policy_by_name("atomic-policy-2").is_none());

    // Commit the staged package
    let versions = engine.commit_staged_package(&staged).unwrap();

    assert_eq!(versions.len(), 2);
    assert!(!engine.is_staging_in_progress());

    // Verify policies ARE visible now
    assert!(engine.get_policy_by_name("atomic-policy-1").is_some());
    assert!(engine.get_policy_by_name("atomic-policy-2").is_some());
}

#[tokio::test]
async fn test_atomic_package_rollback() {
    use crate::data::DataStore;
    use crate::reap::{Decision, Policy as ReapPolicy, PolicyPackage};

    let engine = PolicyEngine::new();
    let store = Arc::new(DataStore::new());

    let policy = ReapPolicy {
        name: "rollback-test-policy".to_string(),
        metadata: std::collections::HashMap::new(),
        default_decision: Decision::Deny,
        rules: vec![],
    };

    let package = PolicyPackage::new(
        "rollback-package".to_string(),
        "1.0.0".to_string(),
        vec![policy],
    );

    // Stage the package
    let _staged = engine.stage_package(&package, store.clone()).unwrap();

    assert!(engine.is_staging_in_progress());

    // Rollback without committing
    engine.rollback_staged();

    assert!(!engine.is_staging_in_progress());
    // Policy should not be visible
    assert!(engine.get_policy_by_name("rollback-test-policy").is_none());
}

#[tokio::test]
async fn test_atomic_deploy_package() {
    use crate::data::DataStore;
    use crate::reap::{Decision, Policy as ReapPolicy, PolicyPackage};

    let engine = PolicyEngine::new();
    let store = Arc::new(DataStore::new());

    let policy = ReapPolicy {
        name: "atomic-deploy-policy".to_string(),
        metadata: std::collections::HashMap::new(),
        default_decision: Decision::Allow,
        rules: vec![],
    };

    let package = PolicyPackage::new(
        "atomic-deploy-package".to_string(),
        "2.0.0".to_string(),
        vec![policy],
    );

    // Deploy atomically (stage + commit in one call)
    let versions = engine
        .deploy_package_atomic(&package, store.clone())
        .unwrap();

    assert_eq!(versions.len(), 1);
    assert!(!engine.is_staging_in_progress());

    // Policy should be visible
    let deployed = engine
        .get_policy_by_name("atomic-deploy-policy")
        .expect("Policy should be deployed");
    assert_eq!(deployed.name, "atomic-deploy-policy");
}

#[tokio::test]
async fn test_concurrent_staging_prevented() {
    use crate::data::DataStore;
    use crate::reap::{Decision, Policy as ReapPolicy, PolicyPackage};

    let engine = PolicyEngine::new();
    let store = Arc::new(DataStore::new());

    let policy1 = ReapPolicy {
        name: "concurrent-policy-1".to_string(),
        metadata: std::collections::HashMap::new(),
        default_decision: Decision::Deny,
        rules: vec![],
    };

    let policy2 = ReapPolicy {
        name: "concurrent-policy-2".to_string(),
        metadata: std::collections::HashMap::new(),
        default_decision: Decision::Allow,
        rules: vec![],
    };

    let package1 = PolicyPackage::new("package-1".to_string(), "1.0.0".to_string(), vec![policy1]);

    let package2 = PolicyPackage::new("package-2".to_string(), "1.0.0".to_string(), vec![policy2]);

    // Stage first package
    let _staged1 = engine.stage_package(&package1, store.clone()).unwrap();

    // Try to stage second package - should fail
    let result = engine.stage_package(&package2, store.clone());
    assert!(result.is_err());

    // Rollback and try again
    engine.rollback_staged();

    let staged2 = engine.stage_package(&package2, store.clone());
    assert!(staged2.is_ok());
}

#[tokio::test]
async fn test_staging_id_mismatch_rejected() {
    use crate::data::DataStore;
    use crate::reap::{Decision, Policy as ReapPolicy, PolicyPackage};

    let engine = PolicyEngine::new();
    let store = Arc::new(DataStore::new());

    let policy = ReapPolicy {
        name: "mismatch-policy".to_string(),
        metadata: std::collections::HashMap::new(),
        default_decision: Decision::Deny,
        rules: vec![],
    };

    let package = PolicyPackage::new(
        "mismatch-package".to_string(),
        "1.0.0".to_string(),
        vec![policy],
    );

    // Stage the package
    let staged = engine.stage_package(&package, store.clone()).unwrap();

    // Create a fake staged package with wrong ID
    let fake_staged = StagedPackage {
        staging_id: uuid::Uuid::new_v4(), // Wrong ID
        staged_policy_ids: staged.staged_policy_ids.clone(),
        staged_policy_names: staged.staged_policy_names.clone(),
        validation_errors: vec![],
        staged_at: chrono::Utc::now(),
    };

    // Try to commit with wrong ID - should fail
    let result = engine.commit_staged_package(&fake_staged);
    assert!(result.is_err());

    // Commit with correct ID should work
    let versions = engine.commit_staged_package(&staged).unwrap();
    assert_eq!(versions.len(), 1);
}

#[test]
fn test_reaper_dsl_policy_rebuilds_evaluator_from_content() {
    // Regression for restart durability: a Reaper-DSL policy persisted with only
    // its `content` (the evaluator is never serialized) must rebuild a working
    // evaluator via build_evaluator_with_data — previously this hard-errored for
    // the old `Custom` language and the policy was silently dropped on restart.
    let content = r#"
policy restart_test {
    default: deny,
    rule allow_read {
        allow if {
            action == "read"
        }
    }
}
"#;

    let mut policy = EnhancedPolicy {
        id: uuid::Uuid::new_v4(),
        version: 1,
        name: "restart_test".to_string(),
        description: String::new(),
        language: crate::PolicyLanguage::ReaperDsl,
        content: content.to_string(),
        rules: vec![],
        metadata: std::collections::HashMap::new(),
        priority: 100,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        evaluator: None,
        source_metadata: None,
    };

    // Rebuild as the agent does on restart, passing the populated DataStore so
    // entity resolution works (the principal must exist as an entity).
    let store = std::sync::Arc::new(crate::data::DataStore::new());
    let loader = crate::data::DataLoader::new((*store).clone());
    loader
        .load_json(r#"{"entities":[{"id":"alice","type":"User","attributes":{"role":"admin"}}]}"#)
        .expect("load entity");

    policy
        .build_evaluator_with_data(Some(store))
        .expect("ReaperDsl policy should rebuild its evaluator, not error");
    assert!(policy.evaluator.is_some());

    let engine = PolicyEngine::new();
    let id = policy.id;
    engine.deploy_policy(policy).unwrap();

    let mut ctx = std::collections::HashMap::new();
    ctx.insert("principal".to_string(), "alice".to_string());
    let allow_req = crate::PolicyRequest {
        resource: "/doc".to_string(),
        action: "read".to_string(),
        context: ctx.clone(),
    };
    let deny_req = crate::PolicyRequest {
        resource: "/doc".to_string(),
        action: "write".to_string(),
        context: ctx,
    };

    assert_eq!(
        engine.evaluate(&id, &allow_req).unwrap().decision,
        PolicyAction::Allow
    );
    assert_eq!(
        engine.evaluate(&id, &deny_req).unwrap().decision,
        PolicyAction::Deny
    );
}

#[test]
fn test_replace_all_policies_is_atomic_full_replace() {
    let engine = PolicyEngine::new();

    // Deploy two policies the normal (additive) way.
    for name in ["policy_a", "policy_b"] {
        let mut p = EnhancedPolicy::new(
            name.to_string(),
            String::new(),
            vec![PolicyRule {
                action: PolicyAction::Allow,
                resource: "*".to_string(),
                conditions: vec![],
            }],
        );
        p.build_evaluator().unwrap();
        engine.deploy_policy(p).unwrap();
    }
    assert_eq!(engine.list_policies().len(), 2);
    assert!(engine.get_policy_by_name("policy_a").is_some());

    // Full-replace with a bundle that contains only policy_c.
    let mut c = EnhancedPolicy::new(
        "policy_c".to_string(),
        String::new(),
        vec![PolicyRule {
            action: PolicyAction::Allow,
            resource: "*".to_string(),
            conditions: vec![],
        }],
    );
    c.build_evaluator().unwrap();
    engine.replace_all_policies(vec![c]).unwrap();

    // Only policy_c remains — the floating a/b were dropped in the swap.
    assert_eq!(engine.list_policies().len(), 1);
    assert!(engine.get_policy_by_name("policy_c").is_some());
    assert!(engine.get_policy_by_name("policy_a").is_none());
    assert!(engine.get_policy_by_name("policy_b").is_none());
}

#[test]
fn test_stable_policy_id_is_deterministic() {
    let a = crate::stable_policy_id("my-policy");
    let b = crate::stable_policy_id("my-policy");
    let c = crate::stable_policy_id("other-policy");
    assert_eq!(
        a, b,
        "same name must map to the same id (idempotent redeploy)"
    );
    assert_ne!(a, c);
}

#[test]
fn test_evaluate_set_production_semantics() {
    // The decision-combination contract shared by the agent's serving path
    // and the control plane's replay engine: default deny, first allow wins,
    // deny overrides, unknown policy id errors -> deny (fail closed).
    let engine = PolicyEngine::new();

    let allow_a = EnhancedPolicy::new(
        "allow-a".to_string(),
        "".to_string(),
        vec![PolicyRule {
            action: PolicyAction::Allow,
            resource: "/a".to_string(),
            conditions: vec![],
        }],
    );
    let deny_all = EnhancedPolicy::new(
        "deny-all".to_string(),
        "".to_string(),
        vec![PolicyRule {
            action: PolicyAction::Deny,
            resource: "*".to_string(),
            conditions: vec![],
        }],
    );
    let (allow_id, deny_id) = (allow_a.id, deny_all.id);
    engine.deploy_policy(allow_a).unwrap();
    engine.deploy_policy(deny_all).unwrap();

    let request = |resource: &str| PolicyRequest {
        resource: resource.to_string(),
        action: "read".to_string(),
        context: Default::default(),
    };

    // Allow-only set: first allow wins, attribution carried.
    let out = engine.evaluate_set(&[allow_id], &request("/a"));
    assert_eq!(out.decision, PolicyAction::Allow);
    assert_eq!(out.policy_name, "allow-a");

    // No rule matched: the only policy is non-decisive (Plan 08 Phase A), so
    // the set falls through to its default deny with nil attribution and no
    // matched rule — the same Deny the agent serves.
    let out = engine.evaluate_set(&[allow_id], &request("/other"));
    assert_eq!(out.decision, PolicyAction::Deny);
    assert_eq!(out.matched_rule, None);
    assert_eq!(out.policy_id, uuid::Uuid::nil());

    // Deny overrides a prior allow, and attribution moves to the denier.
    let out = engine.evaluate_set(&[allow_id, deny_id], &request("/a"));
    assert_eq!(out.decision, PolicyAction::Deny);
    assert_eq!(out.policy_name, "deny-all");

    // Unknown policy id -> error -> deny (fail closed), error surfaced.
    let out = engine.evaluate_set(&[uuid::Uuid::new_v4()], &request("/a"));
    assert_eq!(out.decision, PolicyAction::Deny);
    assert!(out.error.is_some());

    // Empty set -> default deny.
    let out = engine.evaluate_set(&[], &request("/a"));
    assert_eq!(out.decision, PolicyAction::Deny);
}
