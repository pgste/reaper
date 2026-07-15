//! Complex Policy Benchmark Suite
//!
//! Tests combined RBAC+ABAC+ReBAC evaluation, rule scaling,
//! short-circuit effectiveness, condition depth, and AST vs compiled comparison.
//!
//! Groups 1-4 run against the **compiled** evaluator (production path).
//! Group 5 compares AST vs compiled head-to-head across multiple scenarios.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use policy_engine::data::DataStore;
use policy_engine::reap::{ReapAstEvaluator, ReaperPolicy};
use policy_engine::{DataLoader, PolicyAction, PolicyEvaluator, PolicyRequest};
use std::collections::HashMap;
use std::hint::black_box;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_request(principal: &str, action: &str, resource: &str) -> PolicyRequest {
    let mut ctx = HashMap::new();
    ctx.insert("principal".to_string(), principal.to_string());
    PolicyRequest {
        resource: resource.to_string(),
        action: action.to_string(),
        context: ctx,

        ..Default::default()
    }
}

/// Build a compiled evaluator (production path).
fn create_compiled(policy: &str, data: &str) -> Box<dyn PolicyEvaluator> {
    let store = DataStore::new();
    let loader = DataLoader::new(store.clone());
    loader.load_json(data).expect("Failed to load data");
    let p: ReaperPolicy = policy.parse().expect("Failed to parse policy");
    Box::new(p.build(Arc::new(store)).expect("Failed to compile policy"))
}

/// Build both evaluators sharing the same DataStore.
fn create_both(policy_str: &str, data: &str) -> (ReapAstEvaluator, Box<dyn PolicyEvaluator>) {
    let store = DataStore::new();
    let loader = DataLoader::new(store.clone());
    loader.load_json(data).expect("Failed to load data");
    let arc_store = Arc::new(store);

    let ast_eval = {
        let p: ReaperPolicy = policy_str.parse().expect("Failed to parse policy");
        p.build_ast_evaluator(Arc::clone(&arc_store))
    };
    let compiled_eval = {
        let p: ReaperPolicy = policy_str.parse().expect("Failed to parse policy");
        Box::new(p.build(arc_store).expect("Failed to compile policy")) as Box<dyn PolicyEvaluator>
    };
    (ast_eval, compiled_eval)
}

#[inline]
fn eval_compiled(
    ev: &dyn PolicyEvaluator,
    principal: &str,
    action: &str,
    resource: &str,
) -> PolicyAction {
    ev.evaluate(&make_request(principal, action, resource))
        .expect("Eval failed")
}

#[inline]
fn eval_ast(ev: &ReapAstEvaluator, principal: &str, action: &str, resource: &str) -> PolicyAction {
    ev.evaluate(&make_request(principal, action, resource))
        .expect("Eval failed")
}

// ---------------------------------------------------------------------------
// Shared policy / data constants
// ---------------------------------------------------------------------------

const ENTERPRISE_POLICY: &str = r#"
policy enterprise_multilayer {
    description: "Combined RBAC+ABAC+ReBAC enterprise policy",
    default: deny,

    rule suspended_deny {
        deny if user.suspended == true
    }

    rule intern_secret_deny {
        deny if user.role == "intern" && resource.classification == "secret"
    }

    rule admin_allow {
        allow if user.role == "admin"
    }

    rule owner_allow {
        allow if user.id == resource.owner_id && user.status == "active"
    }

    rule team_lead_allow {
        allow if user.team_id == resource.team_id && user.team_role == "lead"
    }

    rule dept_clearance_allow {
        allow if user.department == resource.department && user.clearance >= resource.clearance_required && resource.classification != "secret"
    }

    rule shared_allow {
        allow if user.id == resource.shared_with
    }

    rule executive_allow {
        allow if user.role == "executive" && user.clearance >= 8 && resource.archived != true
    }

    rule public_allow {
        allow if resource.classification == "public" && user.status == "active"
    }
}
"#;

const ENTERPRISE_DATA: &str = r#"{"entities":[
    {"id":"suspended_user","type":"User","attributes":{"suspended":true,"role":"viewer","status":"inactive","department":"hr","clearance":1,"team_id":"t1","team_role":"member","id":"suspended_user"}},
    {"id":"admin_user","type":"User","attributes":{"suspended":false,"role":"admin","status":"active","department":"eng","clearance":10,"team_id":"t2","team_role":"admin","id":"admin_user"}},
    {"id":"owner_user","type":"User","attributes":{"suspended":false,"role":"viewer","status":"active","department":"sales","clearance":3,"team_id":"t3","team_role":"member","id":"owner_user"}},
    {"id":"dept_user","type":"User","attributes":{"suspended":false,"role":"analyst","status":"active","department":"eng","clearance":7,"team_id":"t4","team_role":"member","id":"dept_user"}},
    {"id":"basic_user","type":"User","attributes":{"suspended":false,"role":"viewer","status":"active","department":"marketing","clearance":2,"team_id":"t5","team_role":"member","id":"basic_user"}},
    {"id":"nomatch_user","type":"User","attributes":{"suspended":false,"role":"contractor","status":"pending","department":"external","clearance":1,"team_id":"t99","team_role":"guest","id":"nomatch_user"}},
    {"id":"exec_user","type":"User","attributes":{"suspended":false,"role":"executive","status":"active","department":"exec","clearance":9,"team_id":"t6","team_role":"exec","id":"exec_user"}},
    {"id":"intern_user","type":"User","attributes":{"suspended":false,"role":"intern","status":"active","department":"eng","clearance":1,"team_id":"t2","team_role":"member","id":"intern_user"}},
    {"id":"lead_user","type":"User","attributes":{"suspended":false,"role":"engineer","status":"active","department":"eng","clearance":6,"team_id":"t2","team_role":"lead","id":"lead_user"}},
    {"id":"doc_internal","type":"Resource","attributes":{"classification":"internal","department":"eng","clearance_required":5,"owner_id":"owner_user","team_id":"t2","shared_with":"nobody","archived":false}},
    {"id":"doc_public","type":"Resource","attributes":{"classification":"public","department":"marketing","clearance_required":0,"owner_id":"someone","team_id":"t5","shared_with":"nobody","archived":false}},
    {"id":"doc_secret","type":"Resource","attributes":{"classification":"secret","department":"eng","clearance_required":9,"owner_id":"someone","team_id":"t2","shared_with":"nobody","archived":false}},
    {"id":"doc_archived","type":"Resource","attributes":{"classification":"internal","department":"exec","clearance_required":3,"owner_id":"someone","team_id":"t6","shared_with":"nobody","archived":true}}
]}"#;

// ---------------------------------------------------------------------------
// Group 1: Combined Enterprise Policy — compiled path
// ---------------------------------------------------------------------------

fn combined_enterprise_policy(c: &mut Criterion) {
    let mut group = c.benchmark_group("combined_enterprise");
    let ev = create_compiled(ENTERPRISE_POLICY, ENTERPRISE_DATA);

    // Layer 1 - earliest exit (deny, suspended)
    group.bench_function("suspended_deny", |b| {
        b.iter(|| {
            eval_compiled(
                black_box(ev.as_ref()),
                "suspended_user",
                "read",
                "doc_internal",
            )
        })
    });

    // Layer 3 - early RBAC exit (allow, admin)
    group.bench_function("admin_allow", |b| {
        b.iter(|| eval_compiled(black_box(ev.as_ref()), "admin_user", "read", "doc_internal"))
    });

    // Layer 4 - ReBAC mid-path (allow, owner)
    group.bench_function("owner_allow", |b| {
        b.iter(|| eval_compiled(black_box(ev.as_ref()), "owner_user", "read", "doc_internal"))
    });

    // Layer 6 - ABAC deep path (allow, dept+clearance)
    group.bench_function("dept_clearance_allow", |b| {
        b.iter(|| eval_compiled(black_box(ev.as_ref()), "dept_user", "read", "doc_internal"))
    });

    // Layer 9 - deepest allow (public resource + active user)
    group.bench_function("public_allow", |b| {
        b.iter(|| eval_compiled(black_box(ev.as_ref()), "basic_user", "read", "doc_public"))
    });

    // Default deny - must evaluate ALL rules
    group.bench_function("default_deny", |b| {
        b.iter(|| {
            eval_compiled(
                black_box(ev.as_ref()),
                "nomatch_user",
                "read",
                "doc_internal",
            )
        })
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Group 2: Rule Scaling — compiled path
// ---------------------------------------------------------------------------

fn generate_scaling_policy(num_rules: usize, match_at_end: bool) -> String {
    let mut rules = Vec::new();

    if !match_at_end {
        rules.push("rule match { allow if user.department == resource.department }".to_string());
    }

    for i in 0..num_rules.saturating_sub(1) {
        rules.push(format!(
            "rule filler_{} {{ allow if user.department == \"nonexistent_dept_{}\" }}",
            i, i
        ));
    }

    if match_at_end {
        rules.push("rule match { allow if user.department == resource.department }".to_string());
    }

    format!("policy scaling {{ default: deny, {} }}", rules.join(" "))
}

fn rule_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("rule_scaling");

    let data = r#"{"entities":[
        {"id":"u","type":"User","attributes":{"department":"eng"}},
        {"id":"res","type":"Resource","attributes":{"department":"eng"}}
    ]}"#;

    for num_rules in [1, 2, 5, 10, 25, 50, 100] {
        let policy_best = generate_scaling_policy(num_rules, false);
        let ev = create_compiled(&policy_best, data);
        group.bench_with_input(
            BenchmarkId::new("best_case", num_rules),
            &num_rules,
            |b, _| b.iter(|| eval_compiled(black_box(ev.as_ref()), "u", "read", "res")),
        );

        let policy_worst = generate_scaling_policy(num_rules, true);
        let ev = create_compiled(&policy_worst, data);
        group.bench_with_input(
            BenchmarkId::new("worst_case", num_rules),
            &num_rules,
            |b, _| b.iter(|| eval_compiled(black_box(ev.as_ref()), "u", "read", "res")),
        );
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Group 3: Condition Depth — compiled path
// ---------------------------------------------------------------------------

fn generate_condition_depth_policy(num_conditions: usize) -> String {
    let conditions: Vec<String> = (0..num_conditions)
        .map(|i| format!("user.attr_{} == \"val_{}\"", i, i))
        .collect();
    format!(
        "policy cond_depth {{ default: deny, rule deep {{ allow if {} }} }}",
        conditions.join(" && ")
    )
}

fn generate_condition_depth_data(num_conditions: usize, all_true: bool) -> String {
    let attrs: Vec<String> = (0..num_conditions)
        .map(|i| {
            if all_true || i > 0 {
                format!("\"attr_{}\":\"val_{}\"", i, i)
            } else {
                format!("\"attr_{}\":\"WRONG\"", i)
            }
        })
        .collect();
    format!(
        r#"{{"entities":[{{"id":"u","type":"User","attributes":{{{}}}}},{{"id":"res","type":"Resource","attributes":{{}}}}]}}"#,
        attrs.join(",")
    )
}

fn condition_depth(c: &mut Criterion) {
    let mut group = c.benchmark_group("condition_depth");

    for num_conditions in [1, 2, 4, 8, 16] {
        let policy = generate_condition_depth_policy(num_conditions);

        let data_true = generate_condition_depth_data(num_conditions, true);
        let ev = create_compiled(&policy, &data_true);
        group.bench_with_input(
            BenchmarkId::new("all_true", num_conditions),
            &num_conditions,
            |b, _| b.iter(|| eval_compiled(black_box(ev.as_ref()), "u", "read", "res")),
        );

        let data_false = generate_condition_depth_data(num_conditions, false);
        let ev = create_compiled(&policy, &data_false);
        group.bench_with_input(
            BenchmarkId::new("first_false", num_conditions),
            &num_conditions,
            |b, _| b.iter(|| eval_compiled(black_box(ev.as_ref()), "u", "read", "res")),
        );
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Group 4: Early Exit — compiled path
// ---------------------------------------------------------------------------

fn generate_early_exit_policy(num_allow_rules: usize) -> String {
    let mut rules = Vec::new();

    rules.push("rule block_deny { deny if user.blocked == true }".to_string());
    rules.push("rule suspended_deny { deny if user.status == \"suspended\" }".to_string());

    for i in 0..num_allow_rules {
        rules.push(format!(
            "rule dept_{} {{ allow if user.department == \"dept_{}\" }}",
            i, i
        ));
    }

    rules.push("rule fallback { allow if user.role == \"fallback\" }".to_string());

    format!("policy early_exit {{ default: deny, {} }}", rules.join(" "))
}

fn early_exit(c: &mut Criterion) {
    let mut group = c.benchmark_group("early_exit");

    let data = r#"{"entities":[
        {"id":"blocked_user","type":"User","attributes":{"blocked":true,"status":"active","department":"none","role":"user"}},
        {"id":"fallback_user","type":"User","attributes":{"blocked":false,"status":"active","department":"none","role":"fallback"}},
        {"id":"nomatch_user","type":"User","attributes":{"blocked":false,"status":"active","department":"none","role":"user"}},
        {"id":"res","type":"Resource","attributes":{}}
    ]}"#;

    for num_allow_rules in [10, 25, 50] {
        let policy = generate_early_exit_policy(num_allow_rules);
        let ev = create_compiled(&policy, data);

        group.bench_with_input(
            BenchmarkId::new("deny_at_rule_1", num_allow_rules),
            &num_allow_rules,
            |b, _| b.iter(|| eval_compiled(black_box(ev.as_ref()), "blocked_user", "read", "res")),
        );

        group.bench_with_input(
            BenchmarkId::new("allow_at_last", num_allow_rules),
            &num_allow_rules,
            |b, _| b.iter(|| eval_compiled(black_box(ev.as_ref()), "fallback_user", "read", "res")),
        );

        group.bench_with_input(
            BenchmarkId::new("default_deny", num_allow_rules),
            &num_allow_rules,
            |b, _| b.iter(|| eval_compiled(black_box(ev.as_ref()), "nomatch_user", "read", "res")),
        );
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Group 5: AST vs Compiled — Enterprise policy (9 rules, all layer depths)
// ---------------------------------------------------------------------------

fn ast_vs_compiled_enterprise(c: &mut Criterion) {
    let mut group = c.benchmark_group("ast_vs_compiled_enterprise");
    let (ast, compiled) = create_both(ENTERPRISE_POLICY, ENTERPRISE_DATA);

    let scenarios: &[(&str, &str, &str, &str)] = &[
        // (label, principal, action, resource)
        ("suspended_deny", "suspended_user", "read", "doc_internal"),
        ("intern_secret_deny", "intern_user", "read", "doc_secret"),
        ("admin_allow", "admin_user", "read", "doc_internal"),
        ("owner_allow", "owner_user", "read", "doc_internal"),
        ("team_lead_allow", "lead_user", "read", "doc_internal"),
        ("dept_clearance_allow", "dept_user", "read", "doc_internal"),
        ("executive_allow", "exec_user", "read", "doc_internal"),
        ("public_allow", "basic_user", "read", "doc_public"),
        ("default_deny", "nomatch_user", "read", "doc_internal"),
    ];

    for &(label, principal, action, resource) in scenarios {
        group.bench_function(format!("ast/{}", label), |b| {
            b.iter(|| eval_ast(black_box(&ast), principal, action, resource))
        });
        group.bench_function(format!("compiled/{}", label), |b| {
            b.iter(|| eval_compiled(black_box(compiled.as_ref()), principal, action, resource))
        });
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Group 6: AST vs Compiled — Rule scaling (10, 50, 100 rules worst case)
// ---------------------------------------------------------------------------

fn ast_vs_compiled_rule_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("ast_vs_compiled_rule_scaling");

    let data = r#"{"entities":[
        {"id":"u","type":"User","attributes":{"department":"eng"}},
        {"id":"res","type":"Resource","attributes":{"department":"eng"}}
    ]}"#;

    for num_rules in [10, 50, 100] {
        let policy = generate_scaling_policy(num_rules, true); // worst case
        let (ast, compiled) = create_both(&policy, data);

        group.bench_with_input(
            BenchmarkId::new("ast/worst_case", num_rules),
            &num_rules,
            |b, _| b.iter(|| eval_ast(black_box(&ast), "u", "read", "res")),
        );
        group.bench_with_input(
            BenchmarkId::new("compiled/worst_case", num_rules),
            &num_rules,
            |b, _| b.iter(|| eval_compiled(black_box(compiled.as_ref()), "u", "read", "res")),
        );
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Group 7: AST vs Compiled — Condition depth (4, 8, 16 AND-conditions)
// ---------------------------------------------------------------------------

fn ast_vs_compiled_condition_depth(c: &mut Criterion) {
    let mut group = c.benchmark_group("ast_vs_compiled_condition_depth");

    for num_conditions in [4, 8, 16] {
        let policy = generate_condition_depth_policy(num_conditions);

        // all_true: must evaluate every condition
        let data_true = generate_condition_depth_data(num_conditions, true);
        let (ast, compiled) = create_both(&policy, &data_true);

        group.bench_with_input(
            BenchmarkId::new("ast/all_true", num_conditions),
            &num_conditions,
            |b, _| b.iter(|| eval_ast(black_box(&ast), "u", "read", "res")),
        );
        group.bench_with_input(
            BenchmarkId::new("compiled/all_true", num_conditions),
            &num_conditions,
            |b, _| b.iter(|| eval_compiled(black_box(compiled.as_ref()), "u", "read", "res")),
        );

        // first_false: should short-circuit immediately
        let data_false = generate_condition_depth_data(num_conditions, false);
        let (ast, compiled) = create_both(&policy, &data_false);

        group.bench_with_input(
            BenchmarkId::new("ast/first_false", num_conditions),
            &num_conditions,
            |b, _| b.iter(|| eval_ast(black_box(&ast), "u", "read", "res")),
        );
        group.bench_with_input(
            BenchmarkId::new("compiled/first_false", num_conditions),
            &num_conditions,
            |b, _| b.iter(|| eval_compiled(black_box(compiled.as_ref()), "u", "read", "res")),
        );
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Group 8: AST vs Compiled — Early exit (deny at rule 1 vs full scan, 50 rules)
// ---------------------------------------------------------------------------

fn ast_vs_compiled_early_exit(c: &mut Criterion) {
    let mut group = c.benchmark_group("ast_vs_compiled_early_exit");

    let data = r#"{"entities":[
        {"id":"blocked_user","type":"User","attributes":{"blocked":true,"status":"active","department":"none","role":"user"}},
        {"id":"fallback_user","type":"User","attributes":{"blocked":false,"status":"active","department":"none","role":"fallback"}},
        {"id":"nomatch_user","type":"User","attributes":{"blocked":false,"status":"active","department":"none","role":"user"}},
        {"id":"res","type":"Resource","attributes":{}}
    ]}"#;

    for num_allow_rules in [10, 50] {
        let policy = generate_early_exit_policy(num_allow_rules);
        let (ast, compiled) = create_both(&policy, data);

        group.bench_with_input(
            BenchmarkId::new("ast/deny_at_rule_1", num_allow_rules),
            &num_allow_rules,
            |b, _| b.iter(|| eval_ast(black_box(&ast), "blocked_user", "read", "res")),
        );
        group.bench_with_input(
            BenchmarkId::new("compiled/deny_at_rule_1", num_allow_rules),
            &num_allow_rules,
            |b, _| {
                b.iter(|| {
                    eval_compiled(black_box(compiled.as_ref()), "blocked_user", "read", "res")
                })
            },
        );

        group.bench_with_input(
            BenchmarkId::new("ast/allow_at_last", num_allow_rules),
            &num_allow_rules,
            |b, _| b.iter(|| eval_ast(black_box(&ast), "fallback_user", "read", "res")),
        );
        group.bench_with_input(
            BenchmarkId::new("compiled/allow_at_last", num_allow_rules),
            &num_allow_rules,
            |b, _| {
                b.iter(|| {
                    eval_compiled(black_box(compiled.as_ref()), "fallback_user", "read", "res")
                })
            },
        );

        group.bench_with_input(
            BenchmarkId::new("ast/default_deny", num_allow_rules),
            &num_allow_rules,
            |b, _| b.iter(|| eval_ast(black_box(&ast), "nomatch_user", "read", "res")),
        );
        group.bench_with_input(
            BenchmarkId::new("compiled/default_deny", num_allow_rules),
            &num_allow_rules,
            |b, _| {
                b.iter(|| {
                    eval_compiled(black_box(compiled.as_ref()), "nomatch_user", "read", "res")
                })
            },
        );
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

criterion_group!(
    complex_benches,
    combined_enterprise_policy,
    rule_scaling,
    condition_depth,
    early_exit,
    ast_vs_compiled_enterprise,
    ast_vs_compiled_rule_scaling,
    ast_vs_compiled_condition_depth,
    ast_vs_compiled_early_exit,
);

criterion_main!(complex_benches);
