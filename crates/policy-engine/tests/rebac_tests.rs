//! DSL v2 Phase 3: ReBAC — relationship edges + bounded traversal, composed
//! freely with RBAC and ABAC in single rules.

use policy_engine::reap::ReaperPolicy;
use policy_engine::{DataLoader, DataStore, PolicyRequest};
use std::collections::HashMap;
use std::sync::Arc;

/// Drive-style model: users, groups, folders, docs.
/// alice owns folder-root; eng team can view doc-spec via group membership;
/// docs inherit owners from ancestor folders.
const DATA: &str = r#"{"entities":[
    {"id":"alice","type":"User","attributes":{"role":"engineer","clearance_level":4,"status":"active"},
     "relationships":{"member_of":["team-eng"]}},
    {"id":"bob","type":"User","attributes":{"role":"engineer","clearance_level":1,"status":"active"},
     "relationships":{"member_of":["team-sales"]}},
    {"id":"carol","type":"User","attributes":{"role":"admin","clearance_level":5,"status":"active"}},

    {"id":"team-eng","type":"Group","attributes":{},"relationships":{"member_of":["org-acme"]}},
    {"id":"team-sales","type":"Group","attributes":{}},

    {"id":"folder-root","type":"Folder","attributes":{},"relationships":{"owner":["alice"]}},
    {"id":"folder-eng","type":"Folder","attributes":{},"relationships":{"parent":["folder-root"]}},

    {"id":"doc-spec","type":"Document","attributes":{"clearance_level":3},
     "relationships":{"parent":["folder-eng"],"viewer":["team-eng"]}},
    {"id":"doc-plain","type":"Document","attributes":{"clearance_level":1},
     "relationships":{"parent":["folder-eng"]}}
]}"#;

fn store() -> Arc<DataStore> {
    let store = Arc::new(DataStore::new());
    DataLoader::new((*store).clone())
        .load_json(DATA)
        .expect("load data");
    store
}

fn request(principal: &str, action: &str, resource: &str) -> PolicyRequest {
    let mut context = HashMap::new();
    context.insert("principal".to_string(), principal.to_string());
    PolicyRequest {
        resource: resource.to_string(),
        action: action.to_string(),
        context,

        ..Default::default()
    }
}

fn eval(policy: &str, principal: &str, action: &str, resource: &str) -> String {
    let parsed: ReaperPolicy = policy.parse().expect("parse");
    let evaluator = parsed.build_ast_evaluator(store());
    format!(
        "{:?}",
        evaluator
            .evaluate(&request(principal, action, resource))
            .unwrap()
    )
}

#[test]
fn direct_relation() {
    let policy = r#"
policy owners {
    default: deny,
    rule owner_can_edit {
        allow if rebac::related(user, "owner", resource)
    }
}
"#;
    assert_eq!(eval(policy, "alice", "edit", "folder-root"), "Allow");
    assert_eq!(eval(policy, "bob", "edit", "folder-root"), "Deny");
}

#[test]
fn group_reachable_viewer() {
    // doc-spec#viewer@team-eng; alice --member_of--> team-eng.
    let policy = r#"
policy viewers {
    default: deny,
    rule group_viewer {
        allow if rebac::reachable(user, "viewer", resource, "member_of", 3)
    }
}
"#;
    assert_eq!(eval(policy, "alice", "read", "doc-spec"), "Allow");
    assert_eq!(
        eval(policy, "bob", "read", "doc-spec"),
        "Deny",
        "sales team is not a viewer"
    );
    assert_eq!(
        eval(policy, "alice", "read", "doc-plain"),
        "Deny",
        "no viewer edge at all"
    );
}

#[test]
fn inherited_ownership_through_folders() {
    // alice owns folder-root; doc-spec --parent--> folder-eng --parent--> folder-root.
    let policy = r#"
policy inherit {
    default: deny,
    rule folder_owner_owns_docs {
        allow if rebac::inherited(user, "owner", resource, "parent", 4)
    }
}
"#;
    assert_eq!(eval(policy, "alice", "delete", "doc-spec"), "Allow");
    assert_eq!(eval(policy, "bob", "delete", "doc-spec"), "Deny");

    // Depth bound respected: root is 2 hops up from the doc.
    let shallow = r#"
policy inherit {
    default: deny,
    rule folder_owner_owns_docs {
        allow if rebac::inherited(user, "owner", resource, "parent", 1)
    }
}
"#;
    assert_eq!(eval(shallow, "alice", "delete", "doc-spec"), "Deny");
}

#[test]
fn rbac_abac_rebac_in_one_rule() {
    // THE headline: role (RBAC) + clearance attribute (ABAC) + group
    // relationship (ReBAC) combined in a single condition.
    let policy = r#"
policy combined {
    default: deny,
    rule cleared_engineer_viewers {
        allow if {
            user.role == "engineer" &&
            user.clearance_level >= resource.clearance_level &&
            rebac::reachable(user, "viewer", resource, "member_of", 3)
        }
    }
}
"#;
    // alice: engineer, clearance 4 >= 3, team-eng viewer -> Allow
    assert_eq!(eval(policy, "alice", "read", "doc-spec"), "Allow");
    // bob: engineer but clearance 1 < 3 AND not a viewer -> Deny
    assert_eq!(eval(policy, "bob", "read", "doc-spec"), "Deny");
    // carol: admin (wrong role), high clearance, no membership -> Deny
    assert_eq!(eval(policy, "carol", "read", "doc-spec"), "Deny");
}

#[test]
fn rebac_with_literal_ids_and_bound_variables() {
    // Args are expressions: literals and bound variables work, not just the
    // user/resource pseudo-bindings.
    let policy = r#"
policy literals {
    default: deny,
    rule specific {
        allow if {
            target := "folder-root" &&
            rebac::related(user, "owner", target)
        }
    }
}
"#;
    assert_eq!(eval(policy, "alice", "x", "anything"), "Allow");
    assert_eq!(eval(policy, "bob", "x", "anything"), "Deny");
}

#[test]
fn missing_entities_and_relations_are_total() {
    // Unknown principal/resource/relation: rules fail, evaluation never errors.
    let policy = r#"
policy safe {
    default: deny,
    rule r { allow if rebac::related(user, "nonexistent_relation", resource) }
}
"#;
    assert_eq!(eval(policy, "ghost", "x", "nowhere"), "Deny");
}

// ============================================================================
// Compiled-path coverage: rebac conditions must COMPILE (no AST fallback) and
// return identical decisions to the AST evaluator.
// ============================================================================

#[test]
fn rebac_compiles_to_fast_path_and_matches_ast() {
    use policy_engine::PolicyEvaluator;

    let policy_src = r#"
policy compiled_rebac {
    default: deny,
    rule owner { allow if rebac::related(user, "owner", resource) }
    rule group_viewer { allow if rebac::reachable(user, "viewer", resource, "member_of", 3) }
    rule folder_inherit { allow if rebac::inherited(user, "owner", resource, "parent", 4) }
}
"#;
    let s = store();
    let parsed: ReaperPolicy = policy_src.parse().expect("parse");
    // build() = compiled evaluator; an Err here would mean AST fallback.
    let compiled = parsed
        .clone()
        .build(s.clone())
        .expect("rebac policy must take the COMPILED path");
    let ast = parsed.build_ast_evaluator(s);

    for (principal, resource, expect) in [
        ("alice", "folder-root", "Allow"), // direct owner
        ("alice", "doc-spec", "Allow"),    // group viewer + folder inherit
        ("bob", "doc-spec", "Deny"),
        ("carol", "folder-root", "Deny"),
        ("alice", "doc-plain", "Allow"), // inherited through folders
        ("bob", "folder-root", "Deny"),
    ] {
        let req = request(principal, "read", resource);
        let c = format!("{:?}", compiled.evaluate(&req).unwrap());
        let a = format!("{:?}", ast.evaluate(&req).unwrap());
        assert_eq!(c, expect, "compiled: {principal} -> {resource}");
        assert_eq!(
            c, a,
            "compiled and AST must agree: {principal} -> {resource}"
        );
    }
}

#[test]
fn dynamic_rebac_args_fall_back_to_ast() {
    // A variable object id can't be resolved at compile time: build() must
    // reject it (routing to the AST evaluator), not silently miscompile.
    let policy_src = r#"
policy dynamic {
    default: deny,
    rule r {
        allow if {
            target := "folder-root" &&
            rebac::related(user, "owner", target)
        }
    }
}
"#;
    let parsed: ReaperPolicy = policy_src.parse().expect("parse");
    assert!(
        parsed.clone().build(store()).is_err(),
        "dynamic ids must not compile"
    );
    // ...but the AST evaluator handles it (proven in rebac_with_literal_ids_
    // and_bound_variables above).
}
