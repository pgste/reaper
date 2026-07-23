//! DSL totality (Plan 05, Step 2): a crafted deeply-nested policy is rejected
//! with a typed `InvalidPolicy` error at parse and compile time — never a stack
//! overflow. The parse tests run on a deliberately small thread stack so that a
//! missing guard would abort the thread instead of silently passing.

use std::collections::HashMap;
use std::sync::Arc;

use policy_engine::reap::{compile_policy, Decision, Policy, ReapCondition, ReapParser, ReapRule};
use policy_engine::DataStore;
use reaper_core::ReaperError;

/// Run `f` on a 256 KiB thread stack. Without the depth guard, the recursive
/// descent over 100k-deep input overflows this stack and the join fails; with
/// the guard, `f` returns a clean `Err` far short of it.
fn on_small_stack<F, T>(f: F) -> T
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(f)
        .expect("thread spawns")
        .join()
        .expect("guard returns an error instead of overflowing the stack")
}

#[test]
fn parse_rejects_deep_parens_without_overflow() {
    let src = "(".repeat(100_000);
    let result = on_small_stack(move || ReapParser::parse(&src));
    assert!(
        matches!(result, Err(ReaperError::InvalidPolicy { .. })),
        "expected typed InvalidPolicy, got {result:?}"
    );
}

#[test]
fn parse_rejects_deep_negation_without_overflow() {
    let src = "!".repeat(100_000);
    let result = on_small_stack(move || ReapParser::parse(&src));
    assert!(
        matches!(result, Err(ReaperError::InvalidPolicy { .. })),
        "expected typed InvalidPolicy, got {result:?}"
    );
}

#[test]
fn parse_accepts_a_normal_policy() {
    let src = r#"
        policy example {
            default: deny,
            rule allow_admin { allow if user.role == "admin" }
        }
    "#;
    assert!(ReapParser::parse(src).is_ok());
}

/// A deep AST that reaches the compiler *without* passing the parser's pre-scan
/// (mimicking a YAML/JSON-sourced or directly-constructed policy) is rejected
/// before the recursive compile walk runs. Depth 1000 comfortably exceeds the
/// cap (64) while staying shallow enough that the AST's own recursive `Drop`
/// (which `compile_policy` performs when it consumes the value) is safe — in
/// production serde's own recursion limit prevents a deeper tree ever forming.
#[test]
fn compile_rejects_deep_ast() {
    let mut condition = ReapCondition::True;
    for _ in 0..1_000 {
        condition = ReapCondition::Not(Box::new(condition));
    }
    let policy = Policy {
        name: "deep".to_string(),
        metadata: HashMap::new(),
        default_decision: Decision::Deny,
        rules: vec![ReapRule {
            name: "r".to_string(),
            decision: Decision::Allow,
            condition,
            message: None,
        }],
        functions: vec![],
        imports: vec![],
    };

    let result = compile_policy(policy, Arc::new(DataStore::new()));
    assert!(
        matches!(result, Err(ReaperError::InvalidPolicy { .. })),
        "expected typed InvalidPolicy from compile_policy, got a non-error or wrong variant"
    );
}
