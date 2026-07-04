//! Differential property testing: the compiled evaluator and the AST
//! interpreter MUST agree on every decision, for every policy either can
//! evaluate. This harness generates random-but-valid policies, entity data,
//! relationship graphs, and requests, then asserts parity across the whole
//! space — with proptest shrinking any failure to a minimal counterexample.
//!
//! Why this exists: example-based tests only cover combinations we thought
//! of. Three real miscompiles (context cross-entity comparison, context null
//! checks, and the original library findings) slipped past hundreds of
//! example tests and were caught only when two implementations were run
//! against the same inputs. This suite makes that comparison systematic.
//!
//! Contract enforced here:
//! - If the policy COMPILES: compiled and AST decisions are identical for
//!   every request (both Ok-and-equal; errors must match in kind).
//! - If it does not compile: that's fine (AST fallback is the designed
//!   behavior) — but compilation success/failure must be deterministic.
//!
//! Beyond parity, this file pins SEMANTIC SOUNDNESS:
//! - an independent ORACLE — a deliberately naive, human-auditable
//!   re-implementation of the language semantics over the generated
//!   structures (never touching the production parser/evaluators). All three
//!   implementations must agree, so "both evaluators wrong together" is
//!   caught too.
//! - meta-properties users rely on, tested as laws:
//!   * rule-order invariance (shuffling rules never changes a decision)
//!   * deny monotonicity (adding a deny rule can never turn Deny into Allow)
//!
//! Tuning: PROPTEST_CASES=1000 cargo test -p policy-engine --test
//! differential_parity_tests --release

use policy_engine::reap::ReaperPolicy;
use policy_engine::{DataLoader, DataStore, PolicyEvaluator, PolicyRequest};
use proptest::prelude::*;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Small closed worlds so comparisons hit all outcomes (match, mismatch, absent)
// ---------------------------------------------------------------------------

const USERS: &[&str] = &["alice", "bob", "carol"];
const RESOURCES: &[&str] = &["doc-a", "doc-b", "doc-c"];
const GROUPS: &[&str] = &["g-one", "g-two"];
const ACTIONS: &[&str] = &["read", "write", "delete"];
const ROLES: &[&str] = &["admin", "editor", "viewer"];
const DEPTS: &[&str] = &["eng", "hr"];
const RELATIONS: &[&str] = &["owner", "viewer", "member_of", "parent"];
const CTX_KEYS: &[&str] = &["ticket", "channel"];
const CTX_VALS: &[&str] = &["INC-1", "web"];

/// One randomly generated atomic condition, rendered to .reap source.
#[derive(Debug, Clone)]
enum Atom {
    /// user/resource string attribute vs literal
    AttrEqStr {
        entity: bool,
        attr: &'static str,
        val: &'static str,
        negate: bool,
    },
    /// user/resource numeric attribute vs literal
    AttrCmpNum {
        entity: bool,
        attr: &'static str,
        op: &'static str,
        val: i64,
    },
    /// cross-entity: user.<a> ==/!= resource.<a>
    CrossEq {
        attr: &'static str,
        negate: bool,
    },
    /// context.action == literal
    ActionEq {
        val: &'static str,
    },
    /// context.<key> == literal / != null
    CtxEq {
        key: &'static str,
        val: &'static str,
    },
    CtxNotNull {
        key: &'static str,
    },
    /// "lit" in user.tags
    InArray {
        val: &'static str,
    },
    /// user.badge vs mixed-TYPE literals — pins strict typing (no coercion)
    BadgeEqStr {
        negate: bool,
    }, // user.badge ==/!= "5"
    BadgeEqNum {
        negate: bool,
    }, // user.badge ==/!= 7
    BadgeEqBool {
        negate: bool,
    }, // user.badge ==/!= true
    BadgeOrd {
        op: &'static str,
    }, // user.badge <op> 3 (ordering over non-numbers is false, total)
    /// rebac::related(user, rel, resource)
    RebacDirect {
        rel: &'static str,
    },
    /// rebac::reachable(user, rel, resource, "member_of", d)
    RebacReach {
        rel: &'static str,
        depth: u8,
    },
}

impl Atom {
    fn render(&self) -> String {
        match self {
            Atom::AttrEqStr {
                entity,
                attr,
                val,
                negate,
            } => {
                let e = if *entity { "user" } else { "resource" };
                let op = if *negate { "!=" } else { "==" };
                format!("{e}.{attr} {op} \"{val}\"")
            }
            Atom::AttrCmpNum {
                entity,
                attr,
                op,
                val,
            } => {
                let e = if *entity { "user" } else { "resource" };
                format!("{e}.{attr} {op} {val}")
            }
            Atom::CrossEq { attr, negate } => {
                let op = if *negate { "!=" } else { "==" };
                format!("user.{attr} {op} resource.{attr}")
            }
            Atom::ActionEq { val } => format!("context.action == \"{val}\""),
            Atom::CtxEq { key, val } => format!("context.{key} == \"{val}\""),
            Atom::CtxNotNull { key } => format!("context.{key} != null"),
            Atom::InArray { val } => format!("\"{val}\" in user.tags"),
            Atom::BadgeEqStr { negate } => {
                format!("user.badge {} \"5\"", if *negate { "!=" } else { "==" })
            }
            Atom::BadgeEqNum { negate } => {
                format!("user.badge {} 7", if *negate { "!=" } else { "==" })
            }
            Atom::BadgeEqBool { negate } => {
                format!("user.badge {} true", if *negate { "!=" } else { "==" })
            }
            Atom::BadgeOrd { op } => format!("user.badge {op} 3"),
            Atom::RebacDirect { rel } => {
                format!("rebac::related(user, \"{rel}\", resource)")
            }
            Atom::RebacReach { rel, depth } => {
                format!("rebac::reachable(user, \"{rel}\", resource, \"member_of\", {depth})")
            }
        }
    }
}

fn atom_strategy() -> impl Strategy<Value = Atom> {
    prop_oneof![
        (any::<bool>(), prop::sample::select(ROLES), any::<bool>()).prop_map(
            |(entity, val, negate)| {
                Atom::AttrEqStr {
                    entity,
                    attr: "role",
                    val,
                    negate,
                }
            }
        ),
        (
            any::<bool>(),
            prop::sample::select(&["==", "!=", ">", ">=", "<", "<="][..]),
            0i64..6
        )
            .prop_map(|(entity, op, val)| Atom::AttrCmpNum {
                entity,
                attr: "level",
                op,
                val
            }),
        (prop::sample::select(&["dept"][..]), any::<bool>())
            .prop_map(|(attr, negate)| Atom::CrossEq { attr, negate }),
        prop::sample::select(ACTIONS).prop_map(|val| Atom::ActionEq { val }),
        (
            prop::sample::select(CTX_KEYS),
            prop::sample::select(CTX_VALS)
        )
            .prop_map(|(key, val)| Atom::CtxEq { key, val }),
        prop::sample::select(CTX_KEYS).prop_map(|key| Atom::CtxNotNull { key }),
        prop::sample::select(ROLES).prop_map(|val| Atom::InArray { val }),
        any::<bool>().prop_map(|negate| Atom::BadgeEqStr { negate }),
        any::<bool>().prop_map(|negate| Atom::BadgeEqNum { negate }),
        any::<bool>().prop_map(|negate| Atom::BadgeEqBool { negate }),
        prop::sample::select(&[">", ">=", "<", "<="][..]).prop_map(|op| Atom::BadgeOrd { op }),
        prop::sample::select(&["owner", "viewer"][..]).prop_map(|rel| Atom::RebacDirect { rel }),
        (prop::sample::select(&["owner", "viewer"][..]), 1u8..4)
            .prop_map(|(rel, depth)| Atom::RebacReach { rel, depth }),
    ]
}

/// A rule condition: 1-3 atoms joined by && or ||, optionally negated whole.
#[derive(Debug, Clone)]
struct Cond {
    atoms: Vec<Atom>,
    any: bool, // true = ||, false = &&
}

impl Cond {
    fn render(&self) -> String {
        let joiner = if self.any { " || " } else { " && " };
        let body = self
            .atoms
            .iter()
            .map(Atom::render)
            .collect::<Vec<_>>()
            .join(joiner);
        format!("{{ {body} }}")
    }
}

#[derive(Debug, Clone)]
struct GenPolicy {
    default_allow: bool,
    rules: Vec<(bool /*allow*/, Cond)>,
}

impl GenPolicy {
    fn render(&self) -> String {
        let mut out = String::from("policy diffprop {\n");
        let _ = writeln!(
            out,
            "    default: {},",
            if self.default_allow { "allow" } else { "deny" }
        );
        for (i, (allow, cond)) in self.rules.iter().enumerate() {
            let decision = if *allow { "allow" } else { "deny" };
            let _ = writeln!(out, "    rule r{i} {{ {decision} if {} }}", cond.render());
        }
        out.push('}');
        out
    }
}

fn policy_strategy() -> impl Strategy<Value = GenPolicy> {
    let cond = (prop::collection::vec(atom_strategy(), 1..4), any::<bool>())
        .prop_map(|(atoms, any)| Cond { atoms, any });
    (
        any::<bool>(),
        prop::collection::vec((any::<bool>(), cond), 1..5),
    )
        .prop_map(|(default_allow, rules)| GenPolicy {
            default_allow,
            rules,
        })
}

/// Per-user `badge` attribute whose TYPE varies — pins the type-strict
/// total-comparison contract (no Int/Bool→String coercion; `!=` true only
/// for a present value that differs; ordering only over numbers).
/// 0 = String "5", 1 = Int 2, 2 = Int 7, 3 = Bool true, 4 = absent.
const BADGE_STR5: usize = 0;
const BADGE_INT2: usize = 1;
const BADGE_INT7: usize = 2;
const BADGE_TRUE: usize = 3;
const BADGE_ABSENT: usize = 4;

/// Random world: per-user role/level/dept/tags, per-resource dept/level,
/// random relationship edges.
#[derive(Debug, Clone)]
struct World {
    user_attrs: Vec<(usize, usize, usize, usize)>, // role, level, dept, tag
    badges: Vec<usize>,                            // mixed-TYPE badge per user
    res_attrs: Vec<(usize, usize)>,                // dept, level
    edges: Vec<(usize, usize, usize)>, // resource, relation(owner/viewer), holder idx (user or group)
    memberships: Vec<(usize, usize)>,  // user -> group
    group_nest: bool,                  // g-one member_of g-two
}

fn world_strategy() -> impl Strategy<Value = World> {
    (
        prop::collection::vec(
            (0..ROLES.len(), 0usize..6, 0..DEPTS.len(), 0..ROLES.len()),
            USERS.len()..=USERS.len(),
        ),
        prop::collection::vec(0usize..5, USERS.len()..=USERS.len()),
        prop::collection::vec(
            (0..DEPTS.len(), 0usize..6),
            RESOURCES.len()..=RESOURCES.len(),
        ),
        prop::collection::vec(
            (
                0..RESOURCES.len(),
                0..2usize,
                0..(USERS.len() + GROUPS.len()),
            ),
            0..6,
        ),
        prop::collection::vec((0..USERS.len(), 0..GROUPS.len()), 0..4),
        any::<bool>(),
    )
        .prop_map(
            |(user_attrs, badges, res_attrs, edges, memberships, group_nest)| World {
                user_attrs,
                badges,
                res_attrs,
                edges,
                memberships,
                group_nest,
            },
        )
}

fn build_world(world: &World) -> Arc<DataStore> {
    let mut entities = Vec::new();
    for (i, (role, level, dept, tag)) in world.user_attrs.iter().enumerate() {
        let mut rels = HashMap::new();
        let groups: Vec<String> = world
            .memberships
            .iter()
            .filter(|(u, _)| *u == i)
            .map(|(_, g)| GROUPS[*g].to_string())
            .collect();
        if !groups.is_empty() {
            rels.insert("member_of".to_string(), groups);
        }
        let mut attributes = serde_json::json!({
            "role": ROLES[*role], "level": *level as i64,
            "dept": DEPTS[*dept], "tags": [ROLES[*tag]],
        });
        // Mixed-TYPE badge: string / int / bool / absent.
        let badge = match world.badges[i] {
            BADGE_STR5 => Some(serde_json::json!("5")),
            BADGE_INT2 => Some(serde_json::json!(2)),
            BADGE_INT7 => Some(serde_json::json!(7)),
            BADGE_TRUE => Some(serde_json::json!(true)),
            _ => None,
        };
        if let Some(b) = badge {
            attributes["badge"] = b;
        }
        entities.push(serde_json::json!({
            "id": USERS[i], "type": "User",
            "attributes": attributes,
            "relationships": rels,
        }));
    }
    for (i, (dept, level)) in world.res_attrs.iter().enumerate() {
        let mut rels: HashMap<String, Vec<String>> = HashMap::new();
        for (r, rel, holder) in &world.edges {
            if *r == i {
                let holder_id = if *holder < USERS.len() {
                    USERS[*holder].to_string()
                } else {
                    GROUPS[*holder - USERS.len()].to_string()
                };
                rels.entry(RELATIONS[*rel].to_string())
                    .or_default()
                    .push(holder_id);
            }
        }
        entities.push(serde_json::json!({
            "id": RESOURCES[i], "type": "Resource",
            "attributes": {"dept": DEPTS[*dept], "level": *level as i64},
            "relationships": rels,
        }));
    }
    for (i, g) in GROUPS.iter().enumerate() {
        let mut rels: HashMap<String, Vec<String>> = HashMap::new();
        if world.group_nest && i == 0 {
            rels.insert("member_of".to_string(), vec![GROUPS[1].to_string()]);
        }
        entities.push(serde_json::json!({
            "id": g, "type": "Group", "attributes": {}, "relationships": rels,
        }));
    }

    let doc = serde_json::json!({ "entities": entities });
    let store = Arc::new(DataStore::new());
    DataLoader::new((*store).clone())
        .load_json(&doc.to_string())
        .expect("world loads");
    store
}

fn all_requests() -> Vec<PolicyRequest> {
    let mut requests = Vec::new();
    for user in USERS {
        for resource in RESOURCES {
            for action in ACTIONS {
                // no extra context / with ticket context
                for ctx_extra in [None, Some(("ticket", "INC-1")), Some(("channel", "web"))] {
                    let mut context = HashMap::new();
                    context.insert("principal".to_string(), user.to_string());
                    if let Some((k, v)) = ctx_extra {
                        context.insert(k.to_string(), v.to_string());
                    }
                    requests.push(PolicyRequest {
                        resource: resource.to_string(),
                        action: action.to_string(),
                        context,
                    });
                }
            }
        }
    }
    requests
}

// ---------------------------------------------------------------------------
// The ORACLE: intended language semantics, written to be read and checked by
// a human in one sitting. Deny rules win; then allow rules; then the default.
// Evaluates the GENERATED structures directly — it never touches the parser,
// the AST evaluator, or the compiler, so it cannot share their bugs.
// ---------------------------------------------------------------------------

fn oracle_decide(policy: &GenPolicy, world: &World, request: &PolicyRequest) -> &'static str {
    let matches = |cond: &Cond| -> bool {
        let results = cond.atoms.iter().map(|a| oracle_atom(a, world, request));
        if cond.any {
            results.into_iter().any(|b| b)
        } else {
            results.into_iter().all(|b| b)
        }
    };
    // Phase 1: any matching deny rule denies.
    if policy.rules.iter().any(|(allow, c)| !*allow && matches(c)) {
        return "Deny";
    }
    // Phase 2: any matching allow rule allows.
    if policy.rules.iter().any(|(allow, c)| *allow && matches(c)) {
        return "Allow";
    }
    // Phase 3: default.
    if policy.default_allow {
        "Allow"
    } else {
        "Deny"
    }
}

fn oracle_atom(atom: &Atom, world: &World, request: &PolicyRequest) -> bool {
    let principal = request
        .context
        .get("principal")
        .map(String::as_str)
        .unwrap_or("");
    let u = USERS.iter().position(|x| *x == principal);
    let r = RESOURCES
        .iter()
        .position(|x| *x == request.resource.as_str());

    match atom {
        Atom::AttrEqStr {
            entity,
            attr: _,
            val,
            negate,
        } => {
            let actual = if *entity {
                u.map(|u| ROLES[world.user_attrs[u].0])
            } else {
                // resources have no "role" attribute -> comparison is false
                // (and != of a missing attribute is also false: no value to compare)
                None
            };
            match actual {
                Some(actual) => (actual == *val) != *negate,
                None => false,
            }
        }
        Atom::AttrCmpNum {
            entity,
            attr: _,
            op,
            val,
        } => {
            let actual = if *entity {
                u.map(|u| world.user_attrs[u].1 as i64)
            } else {
                r.map(|r| world.res_attrs[r].1 as i64)
            };
            let Some(actual) = actual else { return false };
            match *op {
                "==" => actual == *val,
                "!=" => actual != *val,
                ">" => actual > *val,
                ">=" => actual >= *val,
                "<" => actual < *val,
                "<=" => actual <= *val,
                _ => unreachable!(),
            }
        }
        Atom::CrossEq { attr: _, negate } => match (u, r) {
            (Some(u), Some(r)) => {
                (DEPTS[world.user_attrs[u].2] == DEPTS[world.res_attrs[r].0]) != *negate
            }
            _ => false,
        },
        Atom::ActionEq { val } => request.action == *val,
        Atom::CtxEq { key, val } => request.context.get(*key).map(String::as_str) == Some(*val),
        Atom::CtxNotNull { key } => request.context.contains_key(*key),
        Atom::InArray { val } => u.is_some_and(|u| ROLES[world.user_attrs[u].3] == *val),
        // Type-strict TOTAL comparisons over the mixed-type badge:
        // == is true only for a same-typed equal value; != is true only for
        // a PRESENT value that differs (a different type differs); ordering
        // is true only over numbers. Absent satisfies nothing.
        Atom::BadgeEqStr { negate } => u.is_some_and(|u| {
            let b = world.badges[u];
            b != BADGE_ABSENT && ((b == BADGE_STR5) != *negate)
        }),
        Atom::BadgeEqNum { negate } => u.is_some_and(|u| {
            let b = world.badges[u];
            b != BADGE_ABSENT && ((b == BADGE_INT7) != *negate)
        }),
        Atom::BadgeEqBool { negate } => u.is_some_and(|u| {
            let b = world.badges[u];
            b != BADGE_ABSENT && ((b == BADGE_TRUE) != *negate)
        }),
        Atom::BadgeOrd { op } => u.is_some_and(|u| {
            let n: i64 = match world.badges[u] {
                BADGE_INT2 => 2,
                BADGE_INT7 => 7,
                _ => return false, // non-numeric or absent: ordering is false
            };
            match *op {
                ">" => n > 3,
                ">=" => n >= 3,
                "<" => n < 3,
                "<=" => n <= 3,
                _ => unreachable!(),
            }
        }),
        Atom::RebacDirect { rel } => match (u, r) {
            (Some(u), Some(r)) => oracle_holders(world, r, rel).contains(&Holder::User(u)),
            _ => false,
        },
        Atom::RebacReach { rel, depth } => match (u, r) {
            (Some(u), Some(r)) => {
                let holders = oracle_holders(world, r, rel);
                if holders.contains(&Holder::User(u)) {
                    return true;
                }
                // groups reachable from the user along member_of, bounded
                let mut reachable: Vec<usize> = Vec::new();
                if *depth >= 1 {
                    for (mu, g) in &world.memberships {
                        if *mu == u && !reachable.contains(g) {
                            reachable.push(*g);
                        }
                    }
                }
                if *depth >= 2
                    && world.group_nest
                    && reachable.contains(&0)
                    && !reachable.contains(&1)
                {
                    reachable.push(1); // g-one member_of g-two
                }
                reachable
                    .iter()
                    .any(|g| holders.contains(&Holder::Group(*g)))
            }
            _ => false,
        },
    }
}

#[derive(PartialEq)]
enum Holder {
    User(usize),
    Group(usize),
}

fn oracle_holders(world: &World, resource: usize, rel: &str) -> Vec<Holder> {
    let rel_idx = RELATIONS.iter().position(|x| *x == rel).unwrap();
    world
        .edges
        .iter()
        .filter(|(res, relation, _)| *res == resource && *relation == rel_idx)
        .map(|(_, _, holder)| {
            if *holder < USERS.len() {
                Holder::User(*holder)
            } else {
                Holder::Group(*holder - USERS.len())
            }
        })
        .collect()
}

/// Explicit `cases:` would silently OVERRIDE the PROPTEST_CASES env var, so
/// scale runs (PROPTEST_CASES=1000) would quietly run the default instead.
/// Read the env ourselves: default 64 worlds x 81 requests = ~5k checks/run.
fn cases_from_env(default: u32) -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: cases_from_env(64),
        max_shrink_iters: 2048,
        ..ProptestConfig::default()
    })]

    #[test]
    fn compiled_and_ast_agree_on_every_decision(
        policy in policy_strategy(),
        world in world_strategy(),
    ) {
        let source = policy.render();
        let parsed: ReaperPolicy = source
            .parse()
            .unwrap_or_else(|e| panic!("generated policy must parse: {e:?}\n{source}"));
        let store = build_world(&world);

        let ast = parsed.clone().build_ast_evaluator(store.clone());
        // Not compiling is legal (AST fallback); miscompiling is not.
        let Ok(compiled) = parsed.build(store) else { return Ok(()); };

        // Order-invariance setup: same policy with rules reversed.
        let mut shuffled = policy.clone();
        shuffled.rules.reverse();
        let shuffled_source = shuffled.render();
        let shuffled_parsed: ReaperPolicy = shuffled_source.parse().expect("shuffle parses");
        let shuffled_ast = shuffled_parsed.build_ast_evaluator(build_world(&world));

        // Deny-monotonicity setup: original policy + one always-true deny rule.
        let mut hardened = policy.clone();
        hardened.rules.push((
            false,
            Cond { atoms: vec![Atom::ActionEq { val: "read" }], any: false },
        ));
        let hardened_source = hardened.render();
        let hardened_parsed: ReaperPolicy = hardened_source.parse().expect("hardened parses");
        let hardened_ast = hardened_parsed.build_ast_evaluator(build_world(&world));

        for request in all_requests() {
            let a = ast.evaluate(&request);
            let c = compiled.evaluate(&request);

            // 1) Compiled/AST parity.
            match (&a, &c) {
                (Ok(ad), Ok(cd)) => {
                    prop_assert_eq!(
                        format!("{ad:?}"), format!("{cd:?}"),
                        "PARITY BREAK\npolicy:\n{}\nrequest: {:?}\nast={:?} compiled={:?}",
                        source, request, ad, cd
                    );
                }
                (Err(_), Err(_)) => {}
                _ => prop_assert!(
                    false,
                    "ERROR-PARITY BREAK\npolicy:\n{}\nrequest: {:?}\nast={:?} compiled={:?}",
                    source, request, a, c
                ),
            }

            let Ok(decision) = a else { continue };
            let got = format!("{decision:?}");

            // 2) Semantic soundness: the naive oracle must agree.
            let expected = oracle_decide(&policy, &world, &request);
            prop_assert_eq!(
                &got, expected,
                "SEMANTICS BREAK (evaluators agree with each other but not the spec)\npolicy:\n{}\nrequest: {:?}",
                source, request
            );

            // 3) Law: rule order never matters.
            let shuffled_decision = format!("{:?}", shuffled_ast.evaluate(&request).unwrap());
            prop_assert_eq!(
                &got, &shuffled_decision,
                "ORDER-DEPENDENCE\npolicy:\n{}\nrequest: {:?}",
                source, request
            );

            // 4) Law: adding a deny rule can never turn Deny into Allow.
            let hardened_decision = format!("{:?}", hardened_ast.evaluate(&request).unwrap());
            prop_assert!(
                !(got == "Deny" && hardened_decision == "Allow"),
                "DENY-MONOTONICITY BREAK\npolicy:\n{}\nrequest: {:?}",
                source, request
            );
        }
    }
}
