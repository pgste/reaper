//! Nesting-depth limits that make the Reaper DSL total and terminating by
//! construction (Plan 05, Step 2; ADR-2).
//!
//! An authorization language must not be able to stack-overflow the parser,
//! compiler, or evaluator on crafted input. The grammar is directly recursive
//! at two points — parenthesised sub-expressions (`primary_expr = "(" ~
//! condition_expr ~ ")"`) and prefix negation (`not_expr = "!" ~ not_expr`) —
//! and pest builds the parse tree by recursive descent, so `"(".repeat(100k)`
//! or `"!".repeat(100k)` recurse at *parse* time before any of our code runs.
//! The `&&`/`||` chains are grammar repetitions, not recursion, so they don't
//! contribute to depth.
//!
//! Two complementary guards:
//!
//! 1. [`source_nesting_exceeds`] — a cheap lexical pre-scan that rejects
//!    over-limit input *before* pest recurses, so pest itself cannot overflow.
//! 2. [`check_policy_depth`] — a self-limiting walk of the built AST (reused by
//!    the compiler and the interpreter) that bounds any tree reaching us
//!    through a non-pest path (the YAML/JSON policy formats, or a hand-built
//!    AST). The walk returns an error at `limit + 1` and unwinds, so it cannot
//!    itself overflow even on a pathologically deep tree.

use super::ast::{
    AssignmentValue, ComparisonLeft, ComparisonRight, Comprehension, Condition, Expr, Policy,
};
use reaper_core::ReaperError;

/// Default maximum syntactic nesting depth. Generous enough for any realistic
/// hand-written policy; a crafted DoS is orders of magnitude past it. Override
/// per deployment with `REAPER_MAX_NESTING_DEPTH` (ADR-2: raise/disable the cap
/// via config, no logic redeploy).
pub const DEFAULT_MAX_NESTING_DEPTH: usize = 64;

/// The nesting cap in effect, honoring the `REAPER_MAX_NESTING_DEPTH` override.
/// Parsed once per call (these paths are not hot); an unparseable or zero value
/// falls back to the default so a typo cannot silently disable the guard.
pub fn configured_max_nesting_depth() -> usize {
    std::env::var("REAPER_MAX_NESTING_DEPTH")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|&v| v > 0)
        .unwrap_or(DEFAULT_MAX_NESTING_DEPTH)
}

fn too_deep(limit: usize) -> ReaperError {
    ReaperError::InvalidPolicy {
        reason: format!(
            "policy nesting depth exceeds the maximum of {limit}; \
             deeply nested conditions/expressions are rejected to keep \
             evaluation total and terminating (set REAPER_MAX_NESTING_DEPTH to adjust)"
        ),
    }
}

/// Reject source whose parenthesis nesting or prefix-negation run exceeds
/// `limit`, scanning without invoking pest so the parser cannot overflow first.
///
/// String literals are skipped so a `"("` *inside* a string never counts. `!=`
/// is the not-equal operator, not a negation, so it does not count either.
/// Parens and negation runs are tracked independently; the worst real recursion
/// is bounded by their sum, which stays comfortably within the stack for any
/// input this accepts.
pub fn source_nesting_exceeds(input: &str, limit: usize) -> bool {
    let bytes = input.as_bytes();
    let mut in_string = false;
    let mut escaped = false;
    let mut paren_depth: usize = 0;
    let mut max_paren: usize = 0;
    let mut bang_run: usize = 0;
    let mut i = 0;

    while i < bytes.len() {
        let c = bytes[i];
        if in_string {
            if escaped {
                escaped = false;
            } else if c == b'\\' {
                escaped = true;
            } else if c == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        match c {
            b'"' => {
                in_string = true;
                bang_run = 0;
            }
            b'(' => {
                paren_depth += 1;
                max_paren = max_paren.max(paren_depth);
                bang_run = 0;
            }
            b')' => {
                paren_depth = paren_depth.saturating_sub(1);
                bang_run = 0;
            }
            b'!' => {
                // `!=` is a comparison operator, not a negation — skip it.
                if bytes.get(i + 1) == Some(&b'=') {
                    bang_run = 0;
                    i += 2;
                    continue;
                }
                bang_run += 1;
                if bang_run > limit {
                    return true;
                }
            }
            // Whitespace keeps an in-progress negation run (`! ! !x` is legal);
            // any other token ends it.
            b' ' | b'\t' | b'\r' | b'\n' => {}
            _ => bang_run = 0,
        }
        if max_paren > limit {
            return true;
        }
        i += 1;
    }
    false
}

/// Convenience wrapper using the configured cap. Returns a typed
/// [`ReaperError::InvalidPolicy`] rather than a bool.
pub fn enforce_source_nesting(input: &str) -> Result<(), ReaperError> {
    let limit = configured_max_nesting_depth();
    if source_nesting_exceeds(input, limit) {
        return Err(too_deep(limit));
    }
    Ok(())
}

/// Validate that no rule's condition/expression tree nests deeper than the
/// configured cap. Cheap on real policies, self-limiting on adversarial ones.
///
/// Func-aware (R4-01 Phase C): also validates the policy's helper-predicate
/// set — names, arities, call-graph DAG (recursion = error) — and accounts
/// call sites at their inline-expanded depth, so a chain of `func` calls
/// cannot smuggle unbounded depth past the cap. Runs on every entry path
/// (pest parser, compiler, AST evaluator `validate()`), covering ASTs that
/// arrive via bundles or hand-built trees.
pub fn enforce_policy_depth(policy: &Policy) -> Result<(), ReaperError> {
    let limit = configured_max_nesting_depth();
    check_policy_depth(policy, limit)?;
    super::functions::validate_policy_functions(policy, limit)
}

/// Walk every rule condition, bounding structural depth at `limit`.
pub fn check_policy_depth(policy: &Policy, limit: usize) -> Result<(), ReaperError> {
    for rule in &policy.rules {
        check_condition_depth(&rule.condition, 0, limit)?;
    }
    Ok(())
}

fn check_condition_depth(cond: &Condition, depth: usize, limit: usize) -> Result<(), ReaperError> {
    if depth > limit {
        return Err(too_deep(limit));
    }
    match cond {
        Condition::True | Condition::False => Ok(()),
        Condition::Comparison { left, right, .. } => {
            check_comparison_side_left(left, depth + 1, limit)?;
            check_comparison_side_right(right, depth + 1, limit)
        }
        Condition::Assignment { value, .. } => check_assignment_depth(value, depth + 1, limit),
        Condition::And(conds) | Condition::Or(conds) => {
            for c in conds {
                check_condition_depth(c, depth + 1, limit)?;
            }
            Ok(())
        }
        Condition::Not(inner) => check_condition_depth(inner, depth + 1, limit),
        Condition::Expr(expr) => check_expr_depth(expr, depth + 1, limit),
    }
}

fn check_comparison_side_left(
    side: &ComparisonLeft,
    depth: usize,
    limit: usize,
) -> Result<(), ReaperError> {
    if depth > limit {
        return Err(too_deep(limit));
    }
    match side {
        ComparisonLeft::Expr(e) => check_expr_depth(e, depth + 1, limit),
        ComparisonLeft::EntityAttr(_) | ComparisonLeft::VarAttr(_) => Ok(()),
    }
}

fn check_comparison_side_right(
    side: &ComparisonRight,
    depth: usize,
    limit: usize,
) -> Result<(), ReaperError> {
    if depth > limit {
        return Err(too_deep(limit));
    }
    match side {
        ComparisonRight::Expr(e) => check_expr_depth(e, depth + 1, limit),
        ComparisonRight::Value(_)
        | ComparisonRight::EntityAttr(_)
        | ComparisonRight::Variable(_)
        | ComparisonRight::VarAttr(_) => Ok(()),
    }
}

fn check_assignment_depth(
    value: &AssignmentValue,
    depth: usize,
    limit: usize,
) -> Result<(), ReaperError> {
    if depth > limit {
        return Err(too_deep(limit));
    }
    match value {
        AssignmentValue::Expr(e) => check_expr_depth(e, depth + 1, limit),
        AssignmentValue::Comparison { left, right, .. } => {
            check_comparison_side_left(left, depth + 1, limit)?;
            check_comparison_side_right(right, depth + 1, limit)
        }
        AssignmentValue::Comprehension(comp) => check_comprehension_depth(comp, depth + 1, limit),
        AssignmentValue::EntityAttr(_)
        | AssignmentValue::Value(_)
        | AssignmentValue::Variable(_) => Ok(()),
    }
}

fn check_comprehension_depth(
    comp: &Comprehension,
    depth: usize,
    limit: usize,
) -> Result<(), ReaperError> {
    if depth > limit {
        return Err(too_deep(limit));
    }
    let (outputs, filters): (Vec<&Expr>, &Vec<Condition>) = match comp {
        Comprehension::Set {
            output, filters, ..
        }
        | Comprehension::Array {
            output, filters, ..
        } => (vec![output], filters),
        Comprehension::Object {
            key,
            value,
            filters,
            ..
        } => (vec![key, value], filters),
    };
    for out in outputs {
        check_expr_depth(out, depth + 1, limit)?;
    }
    for f in filters {
        check_condition_depth(f, depth + 1, limit)?;
    }
    Ok(())
}

fn check_expr_depth(expr: &Expr, depth: usize, limit: usize) -> Result<(), ReaperError> {
    if depth > limit {
        return Err(too_deep(limit));
    }
    match expr {
        Expr::MethodCall { receiver, args, .. } => {
            check_expr_depth(receiver, depth + 1, limit)?;
            for a in args {
                check_expr_depth(a, depth + 1, limit)?;
            }
            Ok(())
        }
        Expr::FunctionCall { args, .. } => {
            for a in args {
                check_expr_depth(a, depth + 1, limit)?;
            }
            Ok(())
        }
        Expr::Literal(_)
        | Expr::Variable(_)
        | Expr::AttributeAccess { .. }
        | Expr::IndexedAccess { .. } => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_scan_flags_deep_parens_and_bangs() {
        assert!(source_nesting_exceeds(&"(".repeat(100_000), 64));
        assert!(source_nesting_exceeds(&"!".repeat(100_000), 64));
        // Exactly at the limit is allowed; one past is not.
        assert!(!source_nesting_exceeds(&"(".repeat(64), 64));
        assert!(source_nesting_exceeds(&"(".repeat(65), 64));
    }

    #[test]
    fn source_scan_ignores_parens_in_strings_and_not_equal() {
        // 100k parens inside a string literal must not trip the guard.
        let s = format!("\"{}\"", "(".repeat(100_000));
        assert!(!source_nesting_exceeds(&s, 64));
        // `!=` chains are operators, not negation.
        assert!(!source_nesting_exceeds(
            &"a != b != c != d".repeat(1000),
            64
        ));
    }

    #[test]
    fn source_scan_allows_realistic_policy() {
        let policy = r#"
            policy example {
                default deny
                rule allow_admins {
                    allow if user.role == "admin" && (user.level > 3 || user.vip == true)
                }
            }
        "#;
        assert!(!source_nesting_exceeds(policy, 64));
    }

    #[test]
    fn ast_walk_bounds_deep_not_chain_without_overflowing() {
        // Hand-build a 100k-deep Not chain (bypasses the pest grammar, mimicking
        // a malicious YAML/JSON-sourced AST). The walk must return an error, not
        // stack-overflow.
        let mut cond = Condition::True;
        for _ in 0..100_000 {
            cond = Condition::Not(Box::new(cond));
        }
        let err = check_condition_depth(&cond, 0, 64).unwrap_err();
        assert!(matches!(err, ReaperError::InvalidPolicy { .. }));

        // Tear the chain down iteratively: `Not(Box<Condition>)` has a recursive
        // Drop that would itself overflow the stack on a 100k-deep value.
        let mut c = cond;
        while let Condition::Not(inner) = c {
            c = *inner;
        }
    }

    #[test]
    fn ast_walk_bounds_deep_method_chain() {
        // roles.count().count()... — a deep MethodCall receiver chain.
        let mut expr = Expr::Variable("roles".to_string());
        for _ in 0..100_000 {
            expr = Expr::MethodCall {
                receiver: Box::new(expr),
                method: super::super::ast::MethodName::Count,
                args: vec![],
            };
        }
        let err = check_expr_depth(&expr, 0, 64).unwrap_err();
        assert!(matches!(err, ReaperError::InvalidPolicy { .. }));

        // Iterative teardown to avoid the recursive-Drop overflow (see above).
        let mut e = expr;
        while let Expr::MethodCall { receiver, .. } = e {
            e = *receiver;
        }
    }

    #[test]
    fn ast_walk_accepts_shallow_tree() {
        let cond = Condition::And(vec![
            Condition::True,
            Condition::Not(Box::new(Condition::False)),
        ]);
        assert!(check_condition_depth(&cond, 0, 64).is_ok());
    }
}
