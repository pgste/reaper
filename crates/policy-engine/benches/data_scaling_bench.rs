//! Data Scaling & Memory Benchmark Suite
//!
//! Measures how entity data volume impacts evaluation performance and memory.
//! Generates data inline (no external files), runs allow and deny scenarios
//! at each scale, reports DataStore memory stats, and prints a projection
//! table estimating limits at larger scales.
//!
//! Groups:
//!   1. data_scaling_allow      — Admin allow path at each N
//!   2. data_scaling_deny       — Default deny (full scan) at each N
//!   3. data_scaling_deny_early — Blocked user deny at rule 1, at each N
//!   4. data_scaling_deep_allow — Department+clearance allow at each N
//!   5. memory_and_projection   — Memory profile + projection table

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use policy_engine::data::{DataStore, DataStoreConfig};
use policy_engine::reap::ReaperPolicy;
use policy_engine::{DataLoader, PolicyAction, PolicyEvaluator, PolicyRequest};
use std::collections::HashMap;
use std::hint::black_box;
use std::sync::Arc;
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const ENTITY_SCALES: &[usize] = &[100, 1_000, 10_000, 50_000, 100_000];

const POLICY_TEXT: &str = r#"
policy data_scaling {
    description: "Data scaling benchmark policy",
    default: deny,

    rule blocked_deny {
        deny if user.blocked == true
    }

    rule admin_allow {
        allow if user.role == "admin"
    }

    rule dept_allow {
        allow if user.department == resource.department && user.clearance >= resource.clearance_required && user.status == "active"
    }
}
"#;

// Role distribution: admin 5%, analyst 20%, engineer 30%, viewer 40%, blocked 5%
const ROLES: &[&str] = &[
    "admin", "analyst", "analyst", "analyst", "analyst", "engineer", "engineer", "engineer",
    "engineer", "engineer", "engineer", "viewer", "viewer", "viewer", "viewer", "viewer", "viewer",
    "viewer", "viewer", "blocked",
];

const DEPARTMENTS: &[&str] = &["eng", "sales", "hr", "marketing", "ops"];

const CLASSIFICATIONS: &[&str] = &["public", "internal", "confidential", "secret"];

// ---------------------------------------------------------------------------
// Data generation
// ---------------------------------------------------------------------------

fn generate_data(num_entities: usize) -> String {
    let num_users = num_entities / 2;
    let num_resources = num_entities - num_users;

    let mut entities = Vec::with_capacity(num_entities);

    for i in 0..num_users {
        let role = ROLES[i % ROLES.len()];
        let dept = DEPARTMENTS[i % DEPARTMENTS.len()];
        let clearance = (i % 10) + 1;
        let status = if role == "blocked" {
            "inactive"
        } else {
            "active"
        };
        let blocked = role == "blocked";

        entities.push(format!(
            r#"{{"id":"user_{}","type":"User","attributes":{{"role":"{}","department":"{}","clearance":{},"status":"{}","blocked":{},"id":"user_{}"}}}}"#,
            i, role, dept, clearance, status, blocked, i
        ));
    }

    for i in 0..num_resources {
        let dept = DEPARTMENTS[i % DEPARTMENTS.len()];
        let classification = CLASSIFICATIONS[i % CLASSIFICATIONS.len()];
        let clearance_required = (i % 9) + 1;
        let owner_idx = i % num_users;

        entities.push(format!(
            r#"{{"id":"res_{}","type":"Resource","attributes":{{"department":"{}","classification":"{}","clearance_required":{},"owner_id":"user_{}"}}}}"#,
            i, dept, classification, clearance_required, owner_idx
        ));
    }

    format!(r#"{{"entities":[{}]}}"#, entities.join(","))
}

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

fn build_evaluator(data_json: &str) -> Box<dyn PolicyEvaluator> {
    let store = DataStore::new();
    let loader = DataLoader::new(store.clone());
    loader.load_json(data_json).expect("Failed to load data");
    let policy: ReaperPolicy = POLICY_TEXT.parse().expect("Failed to parse policy");
    Box::new(
        policy
            .build(Arc::new(store))
            .expect("Failed to compile policy"),
    )
}

fn build_evaluator_no_index(data_json: &str) -> Box<dyn PolicyEvaluator> {
    let store = DataStore::with_config(DataStoreConfig {
        index_attributes: false,
        index_composite: false,
    });
    let loader = DataLoader::new(store.clone());
    loader.load_json(data_json).expect("Failed to load data");
    let policy: ReaperPolicy = POLICY_TEXT.parse().expect("Failed to parse policy");
    Box::new(
        policy
            .build(Arc::new(store))
            .expect("Failed to compile policy"),
    )
}

#[inline]
fn eval(ev: &dyn PolicyEvaluator, principal: &str, action: &str, resource: &str) -> PolicyAction {
    ev.evaluate(&make_request(principal, action, resource))
        .expect("Eval failed")
}

/// Configure Criterion group for large entity counts to keep bench time reasonable.
fn configure_for_scale(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    n: usize,
) {
    if n >= 50_000 {
        group.sample_size(10);
        group.measurement_time(Duration::from_secs(3));
    } else if n >= 10_000 {
        group.sample_size(20);
        group.measurement_time(Duration::from_secs(3));
    }
}

// ---------------------------------------------------------------------------
// Group 1: data_scaling_allow — Admin allow path
// ---------------------------------------------------------------------------

fn data_scaling_allow(c: &mut Criterion) {
    let mut group = c.benchmark_group("data_scaling_allow");

    for &n in ENTITY_SCALES {
        configure_for_scale(&mut group, n);
        let data = generate_data(n);
        let ev = build_evaluator(&data);

        // user_0 has role="admin" (first in ROLES cycle)
        group.bench_with_input(BenchmarkId::new("admin_allow", n), &n, |b, _| {
            b.iter(|| eval(black_box(ev.as_ref()), "user_0", "read", "res_0"))
        });
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Group 2: data_scaling_deny — Default deny (full rule scan)
// ---------------------------------------------------------------------------

fn data_scaling_deny(c: &mut Criterion) {
    let mut group = c.benchmark_group("data_scaling_deny");

    for &n in ENTITY_SCALES {
        configure_for_scale(&mut group, n);
        let data = generate_data(n);
        let ev = build_evaluator(&data);

        // user_3 is analyst in "marketing" dept; res_0 is in "eng" dept
        // No rules match: not blocked, not admin, dept mismatch → default deny
        group.bench_with_input(BenchmarkId::new("default_deny", n), &n, |b, _| {
            b.iter(|| eval(black_box(ev.as_ref()), "user_3", "read", "res_0"))
        });
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Group 3: data_scaling_deny_early — Blocked user, deny at rule 1
// ---------------------------------------------------------------------------

fn data_scaling_deny_early(c: &mut Criterion) {
    let mut group = c.benchmark_group("data_scaling_deny_early");

    for &n in ENTITY_SCALES {
        configure_for_scale(&mut group, n);
        let data = generate_data(n);
        let ev = build_evaluator(&data);

        // user_19 has role="blocked" (index 19 % 20 = 19, last in ROLES = "blocked")
        // Should hit blocked_deny rule immediately
        group.bench_with_input(BenchmarkId::new("blocked_deny", n), &n, |b, _| {
            b.iter(|| eval(black_box(ev.as_ref()), "user_19", "read", "res_0"))
        });
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Group 4: data_scaling_deep_allow — Department+clearance allow
// ---------------------------------------------------------------------------

fn data_scaling_deep_allow(c: &mut Criterion) {
    let mut group = c.benchmark_group("data_scaling_deep_allow");

    for &n in ENTITY_SCALES {
        configure_for_scale(&mut group, n);
        let data = generate_data(n);
        let ev = build_evaluator(&data);

        // user_5 is engineer (idx 5 % 20 = 5 → "engineer"), dept = "eng" (5 % 5 = 0),
        //   clearance = (5 % 10) + 1 = 6, status = "active"
        // res_0 has dept = "eng" (0 % 5 = 0), clearance_required = (0 % 9) + 1 = 1
        // dept matches, clearance 6 >= 1, status active → dept_allow fires
        group.bench_with_input(BenchmarkId::new("dept_clearance_allow", n), &n, |b, _| {
            b.iter(|| eval(black_box(ev.as_ref()), "user_5", "read", "res_0"))
        });
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Group 5: data_scaling_allow_no_index — Admin allow with secondary indexes disabled
// ---------------------------------------------------------------------------

fn data_scaling_allow_no_index(c: &mut Criterion) {
    let mut group = c.benchmark_group("data_scaling_allow_no_index");

    for &n in ENTITY_SCALES {
        configure_for_scale(&mut group, n);
        let data = generate_data(n);
        let ev = build_evaluator_no_index(&data);

        group.bench_with_input(BenchmarkId::new("admin_allow", n), &n, |b, _| {
            b.iter(|| eval(black_box(ev.as_ref()), "user_0", "read", "res_0"))
        });
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Group 6: memory_and_projection — Memory profile + projection table
// ---------------------------------------------------------------------------

struct ScalePoint {
    entities: usize,
    memory_bytes: usize,
    bytes_per_entity: f64,
    unique_strings: usize,
    load_ms: f64,
    eval_ns: f64,
}

fn memory_and_projection(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_and_projection");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(1));

    // Single benchmark function that collects all scale points and prints the table
    group.bench_function("profile", |b| {
        b.iter(|| {
            let mut points: Vec<ScalePoint> = Vec::new();

            for &n in ENTITY_SCALES {
                let data_json = generate_data(n);

                // Measure load time
                let load_start = Instant::now();
                let store = DataStore::new();
                let loader = DataLoader::new(store.clone());
                loader.load_json(&data_json).expect("Failed to load data");
                let load_ms = load_start.elapsed().as_secs_f64() * 1000.0;

                // Capture stats
                let stats = store.stats();

                // Build evaluator and measure eval latency
                let policy: ReaperPolicy = POLICY_TEXT.parse().expect("Failed to parse policy");
                let ev = policy.build(Arc::new(store)).expect("Failed to compile policy");

                let req = make_request("user_0", "read", "res_0");
                let eval_start = Instant::now();
                let num_evals = 10_000;
                for _ in 0..num_evals {
                    let _ = black_box(ev.evaluate(black_box(&req)));
                }
                let eval_ns = eval_start.elapsed().as_nanos() as f64 / num_evals as f64;

                let bytes_per_entity = if stats.total_entities > 0 {
                    stats.estimated_memory_bytes as f64 / stats.total_entities as f64
                } else {
                    0.0
                };

                points.push(ScalePoint {
                    entities: stats.total_entities,
                    memory_bytes: stats.estimated_memory_bytes,
                    bytes_per_entity,
                    unique_strings: stats.interner_stats.unique_strings,
                    load_ms,
                    eval_ns,
                });
            }

            black_box(&points);

            // Compute projection from the 3 largest measured points
            let measured_len = points.len();
            let avg_bpe = if measured_len >= 3 {
                let sum: f64 = points[measured_len - 3..]
                    .iter()
                    .map(|p| p.bytes_per_entity)
                    .sum();
                sum / 3.0
            } else {
                points.last().map(|p| p.bytes_per_entity).unwrap_or(120.0)
            };

            // Projection ratios from last two points
            let (load_ratio, eval_ratio, string_ratio) = if measured_len >= 2 {
                let p1 = &points[measured_len - 2];
                let p2 = &points[measured_len - 1];
                let entity_ratio = p2.entities as f64 / p1.entities as f64;
                (
                    p2.load_ms / p1.load_ms,
                    if p1.eval_ns > 0.0 { p2.eval_ns / p1.eval_ns } else { 1.0 },
                    if entity_ratio > 0.0 {
                        (p2.unique_strings as f64 / p1.unique_strings as f64) / entity_ratio
                    } else {
                        1.0
                    },
                )
            } else {
                (2.0, 1.0, 1.0)
            };

            // Print the projection table
            eprintln!();
            eprintln!("\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}");
            eprintln!("                    DATA SCALING MEMORY PROFILE");
            eprintln!("\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}");
            eprintln!(
                "{:<12}{:<12}{:<12}{:<10}{:<10}{:<12}",
                "Entities", "Memory", "Bytes/Ent", "Strings", "Load ms", "Eval ns"
            );
            eprintln!("{}", "\u{2500}".repeat(67));

            // Measured points
            for p in &points {
                eprintln!(
                    "{:<12}{:<12}{:<12}{:<10}{:<10}{:<12}",
                    format_entities(p.entities),
                    format_memory(p.memory_bytes),
                    format!("{:.0} B", p.bytes_per_entity),
                    format_entities(p.unique_strings),
                    format!("{:.1}", p.load_ms),
                    format!("{:.0}", p.eval_ns),
                );
            }

            eprintln!("{}", "\u{2500}".repeat(67));

            // Projected points
            let last = points.last().unwrap();
            let projected_scales: &[usize] = &[500_000, 1_000_000];
            for &target in projected_scales {
                let scale = target as f64 / last.entities as f64;
                let proj_memory = (avg_bpe * target as f64) as usize;
                let proj_load = last.load_ms * scale.powf(load_ratio.log2() / (last.entities as f64 / points[measured_len - 2].entities as f64).log2());
                let proj_eval = last.eval_ns * scale.powf(eval_ratio.log2() / (last.entities as f64 / points[measured_len - 2].entities as f64).log2());
                let proj_strings = (last.unique_strings as f64 * scale.powf(string_ratio)) as usize;

                eprintln!(
                    "{:<12}{:<12}{:<12}{:<10}{:<10}{:<12}  PROJECTED",
                    format_entities(target),
                    format!("~{}", format_memory(proj_memory)),
                    format!("~{:.0} B", avg_bpe),
                    format!("~{}", format_entities(proj_strings)),
                    format!("~{:.0}", proj_load),
                    format!("~{:.0}", proj_eval),
                );
            }

            eprintln!("{}", "\u{2550}".repeat(67));
            eprintln!();
            eprintln!("Scaling model: linear (bytes/entity stable)");
            eprintln!("Memory per entity: ~{:.0} bytes (mean across largest 3 points)", avg_bpe);

            let cap_50mb = (50.0 * 1024.0 * 1024.0 / avg_bpe) as usize;
            let cap_256mb = (256.0 * 1024.0 * 1024.0 / avg_bpe) as usize;
            let cap_1gb = (1024.0 * 1024.0 * 1024.0 / avg_bpe) as usize;
            eprintln!("Entity capacity at  50 MB: ~{}", format_entities(cap_50mb));
            eprintln!("Entity capacity at 256 MB: ~{}", format_entities(cap_256mb));
            eprintln!("Entity capacity at   1 GB: ~{}", format_entities(cap_1gb));
            eprintln!();

            // No-index comparison
            eprintln!("\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}");
            eprintln!("              NO-INDEX MEMORY COMPARISON (indexes disabled)");
            eprintln!("\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}");
            eprintln!(
                "{:<12}{:<12}{:<12}{:<12}{:<12}{:<12}",
                "Entities", "Indexed", "No-Index", "B/E Idx", "B/E NoIdx", "Savings"
            );
            eprintln!("{}", "\u{2500}".repeat(67));

            for &n in ENTITY_SCALES {
                let data_json = generate_data(n);

                let store_idx = DataStore::new();
                let loader_idx = DataLoader::new(store_idx.clone());
                loader_idx.load_json(&data_json).expect("Failed to load data");
                let stats_idx = store_idx.stats();

                let store_noidx = DataStore::with_config(DataStoreConfig {
                    index_attributes: false,
                    index_composite: false,
                });
                let loader_noidx = DataLoader::new(store_noidx.clone());
                loader_noidx.load_json(&data_json).expect("Failed to load data");
                let stats_noidx = store_noidx.stats();

                let bpe_idx = stats_idx.estimated_memory_bytes as f64 / stats_idx.total_entities.max(1) as f64;
                let bpe_noidx = stats_noidx.estimated_memory_bytes as f64 / stats_noidx.total_entities.max(1) as f64;
                let savings_pct = if bpe_idx > 0.0 { (1.0 - bpe_noidx / bpe_idx) * 100.0 } else { 0.0 };

                eprintln!(
                    "{:<12}{:<12}{:<12}{:<12}{:<12}{:<12}",
                    format_entities(n),
                    format_memory(stats_idx.estimated_memory_bytes),
                    format_memory(stats_noidx.estimated_memory_bytes),
                    format!("{:.0} B", bpe_idx),
                    format!("{:.0} B", bpe_noidx),
                    format!("{:.1}%", savings_pct),
                );
            }

            eprintln!("{}", "\u{2550}".repeat(67));
            eprintln!();
        });
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

fn format_entities(n: usize) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        format!("{}", n)
    }
}

fn format_memory(bytes: usize) -> String {
    let mb = bytes as f64 / (1024.0 * 1024.0);
    if mb >= 1.0 {
        format!("{:.2} MB", mb)
    } else {
        let kb = bytes as f64 / 1024.0;
        format!("{:.1} KB", kb)
    }
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

criterion_group!(
    data_scaling_benches,
    data_scaling_allow,
    data_scaling_deny,
    data_scaling_deny_early,
    data_scaling_deep_allow,
    data_scaling_allow_no_index,
    memory_and_projection,
);

criterion_main!(data_scaling_benches);
