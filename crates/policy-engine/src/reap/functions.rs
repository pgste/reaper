//! Helper-predicate (`func`) analysis: name validation, call-graph DAG
//! enforcement, and inline-effective depth accounting (R4-01 Phase C).
//!
//! The `.reap` DSL stays total and terminating by construction: a `func` may
//! call other `func`s but the call graph must be a DAG (recursion — direct or
//! mutual — is a parse/validate error, never a runtime loop), and the nesting
//! cap applies to the *inline-expanded* tree, so a chain of calls cannot
//! smuggle unbounded depth past the per-tree limit. This module runs from
//! [`crate::reap::limits::enforce_policy_depth`], which every entry to
//! evaluation already calls (the pest parser, the compiler, and the AST
//! evaluator's `validate()`), so ASTs arriving via non-parser paths (bundles,
//! YAML/JSON, hand-built trees) get the same guarantees.

use super::ast::{
    AssignmentValue, ComparisonLeft, ComparisonRight, Comprehension, Condition, Expr, FuncDef,
    Policy,
};
use reaper_core::ReaperError;
use std::collections::HashSet;

/// Builtin function namespaces. An import alias or imported-function
/// namespace may not collide with these — `time::x(...)` must always mean the
/// builtin namespace.
pub(crate) const BUILTIN_NAMESPACES: &[&str] =
    &["time", "math", "regex", "json", "jwt", "rebac", "taint"];

/// Builtin global (un-namespaced) functions. A policy-local `func` may not
/// take one of these names.
pub(crate) const BUILTIN_GLOBALS: &[&str] = &[
    "concat",
    "is_string",
    "is_number",
    "is_bool",
    "is_array",
    "is_set",
    "is_object",
    "is_null",
];

/// Identifiers that can never name a `func` or a parameter (entity keywords,
/// literals, and structural keywords).
const RESERVED_NAMES: &[&str] = &[
    "user", "actor", "resource", "context", "input", "true", "false", "null", "func", "rule",
    "policy", "library", "import", "default", "allow", "deny",
];

/// Find the position of `(namespace, function)` in a function slice.
pub(crate) fn find_function(
    functions: &[FuncDef],
    namespace: Option<&str>,
    function: &str,
) -> Option<usize> {
    functions
        .iter()
        .position(|f| f.namespace.as_deref() == namespace && f.name == function)
}

/// Result of analyzing a policy's function set: each function's
/// inline-effective depth (the depth its body contributes when a call to it
/// expands in place).
pub(crate) struct FuncAnalysis {
    effective_depth: Vec<usize>,
}

impl FuncAnalysis {
    fn empty() -> Self {
        Self {
            effective_depth: Vec::new(),
        }
    }
}

/// Validate a policy's function set, compute inline-effective depths, and
/// depth-check every rule condition with call expansions accounted.
///
/// Checks, in order:
/// - function names: not reserved, no collision with builtin globals (local
///   funcs) or builtin namespaces (imported funcs), no duplicates;
/// - parameter lists: no duplicates, no reserved names, no param shadowed by
///   a variable the body binds (keeps inline alpha-renaming trivially sound);
/// - every call to a defined function has matching arity; calls to unknown
///   namespaces (not builtin, not a defined function namespace, not a
///   declared import alias) are rejected;
/// - the call graph over user functions is a DAG (recursion = error);
/// - every function's inline-effective depth, and every rule condition with
///   calls expanded, is within `limit`.
///
/// Calls to a DECLARED import alias whose functions are not yet merged are
/// deferred (the load path re-validates after resolution — see
/// [`verify_resolved_calls`]).
pub(crate) fn validate_policy_functions(policy: &Policy, limit: usize) -> Result<(), ReaperError> {
    let analysis = analyze(policy, limit, true)?;
    check_rules(policy, limit, &analysis, true)
}

/// Strict post-import-resolution pass: like [`validate_policy_functions`] but
/// with no declared-alias deferral — every non-builtin namespaced call must
/// now resolve to a merged function. Run by the load path after imports merge.
pub(crate) fn verify_resolved_calls(policy: &Policy, limit: usize) -> Result<(), ReaperError> {
    let analysis = analyze(policy, limit, false)?;
    check_rules(policy, limit, &analysis, false)
}

fn analyze(
    policy: &Policy,
    limit: usize,
    defer_declared_aliases: bool,
) -> Result<FuncAnalysis, ReaperError> {
    if policy.functions.is_empty() && policy.imports.is_empty() {
        return Ok(FuncAnalysis::empty());
    }

    let mut seen: HashSet<(Option<&str>, &str)> = HashSet::new();
    for f in &policy.functions {
        if RESERVED_NAMES.contains(&f.name.as_str()) {
            return Err(invalid(format!(
                "func name '{}' is reserved and cannot be used",
                f.name
            )));
        }
        match &f.namespace {
            None => {
                if BUILTIN_GLOBALS.contains(&f.name.as_str()) {
                    return Err(invalid(format!(
                        "func name '{}' collides with a builtin function",
                        f.name
                    )));
                }
            }
            Some(ns) => {
                if BUILTIN_NAMESPACES.contains(&ns.as_str()) {
                    return Err(invalid(format!(
                        "function namespace '{ns}' collides with a builtin namespace"
                    )));
                }
            }
        }
        let mut seen_params = HashSet::new();
        for p in &f.params {
            if RESERVED_NAMES.contains(&p.as_str()) {
                return Err(invalid(format!(
                    "func '{}' parameter '{}' is a reserved name",
                    f.name, p
                )));
            }
            if !seen_params.insert(p.as_str()) {
                return Err(invalid(format!(
                    "func '{}' declares parameter '{}' more than once",
                    f.name, p
                )));
            }
        }
        // A body-bound variable shadowing a parameter would make the
        // parameter unreachable past the binding and complicate inlining's
        // alpha-rename; reject the confusing shape outright.
        let mut bound = HashSet::new();
        collect_bound_vars_condition(&f.body, &mut bound);
        for p in &f.params {
            if bound.contains(p.as_str()) {
                return Err(invalid(format!(
                    "func '{}' binds a variable that shadows its parameter '{}'",
                    f.name, p
                )));
            }
        }
        // No free variables: a func is a pure predicate over its parameters
        // and the request entities. A body referencing a variable that is
        // neither a parameter nor body-bound would read the CALLER's rule
        // scope under inlining but be undefined under interpretation — the
        // divergence is unrepresentable because we reject it here.
        let mut referenced = HashSet::new();
        collect_referenced_vars_condition(&f.body, &mut referenced);
        for r in &referenced {
            let allowed = f.params.iter().any(|p| p == r)
                || bound.contains(r.as_str())
                || PSEUDO_VARS.contains(&r.as_str())
                || is_entity_rooted_name(r);
            if !allowed {
                return Err(invalid(format!(
                    "func '{}' references undefined variable '{}' (function bodies \
                     may only use parameters, variables they bind, and the request \
                     entities)",
                    f.name, r
                )));
            }
        }
        if !seen.insert((f.namespace.as_deref(), f.name.as_str())) {
            return Err(invalid(format!(
                "duplicate func definition '{}'",
                qualified(f)
            )));
        }
    }

    let ctx = MeasureCtx {
        limit,
        functions: &policy.functions,
        declared_aliases: if defer_declared_aliases {
            policy.imports.iter().map(|i| i.alias.as_str()).collect()
        } else {
            HashSet::new()
        },
    };

    // Per-function: structural max depth + call sites (site depth, callee).
    let analysis_input: Vec<(usize, Vec<(usize, usize)>)> = policy
        .functions
        .iter()
        .map(|f| {
            let mut sites = Vec::new();
            let max = ctx.measure_condition(&f.body, 0, &mut sites)?;
            Ok((max, sites))
        })
        .collect::<Result<_, ReaperError>>()?;

    // DAG check + inline-effective depth, bottom-up. The recursion is bounded:
    // every call node sits at structural depth >= 1, so effective depth grows
    // by at least 1 per call-chain level and a chain longer than `limit`
    // errors before the stack can get deep.
    #[derive(Clone, Copy, PartialEq)]
    enum State {
        Unvisited,
        Visiting,
        Done,
    }
    struct Dfs<'a> {
        input: &'a [(usize, Vec<(usize, usize)>)],
        funcs: &'a [FuncDef],
        state: Vec<State>,
        effective: Vec<usize>,
        limit: usize,
    }
    impl Dfs<'_> {
        fn visit(&mut self, i: usize, chain: usize) -> Result<usize, ReaperError> {
            match self.state[i] {
                State::Done => return Ok(self.effective[i]),
                State::Visiting => {
                    return Err(invalid(format!(
                        "func '{}' is recursive (directly or through other functions); \
                         recursive functions are not permitted — the call graph must be a DAG",
                        qualified(&self.funcs[i])
                    )));
                }
                State::Unvisited => {}
            }
            if chain > self.limit {
                return Err(too_deep(self.limit));
            }
            self.state[i] = State::Visiting;
            let (structural, ref sites) = self.input[i];
            let mut eff = structural;
            for &(site_depth, callee) in sites {
                let callee_eff = self.visit(callee, chain + 1)?;
                eff = eff.max(site_depth + callee_eff);
            }
            if eff > self.limit {
                return Err(too_deep(self.limit));
            }
            self.state[i] = State::Done;
            self.effective[i] = eff;
            Ok(eff)
        }
    }
    let n = policy.functions.len();
    let mut dfs = Dfs {
        input: &analysis_input,
        funcs: &policy.functions,
        state: vec![State::Unvisited; n],
        effective: vec![0; n],
        limit,
    };
    for i in 0..n {
        dfs.visit(i, 0)?;
    }

    Ok(FuncAnalysis {
        effective_depth: dfs.effective,
    })
}

/// Depth-check every rule condition with function calls accounted at their
/// inline-effective depth: a call node at depth `d` to a function of
/// effective depth `e` behaves as a subtree bottoming out at `d + e`.
fn check_rules(
    policy: &Policy,
    limit: usize,
    analysis: &FuncAnalysis,
    defer_declared_aliases: bool,
) -> Result<(), ReaperError> {
    if policy.functions.is_empty() && policy.imports.is_empty() {
        return Ok(());
    }
    let ctx = MeasureCtx {
        limit,
        functions: &policy.functions,
        declared_aliases: if defer_declared_aliases {
            policy.imports.iter().map(|i| i.alias.as_str()).collect()
        } else {
            HashSet::new()
        },
    };
    for rule in &policy.rules {
        let mut sites = Vec::new();
        ctx.measure_condition(&rule.condition, 0, &mut sites)?;
        for (site_depth, callee) in sites {
            if site_depth + analysis.effective_depth[callee] > limit {
                return Err(too_deep(limit));
            }
        }
        // Message expressions may not call user functions (messages render
        // values; helpers are predicates) — checked via the same walk.
        if let Some(msg) = &rule.message {
            let mut msg_sites = Vec::new();
            ctx.measure_expr(msg, 0, &mut msg_sites)?;
            if !msg_sites.is_empty() {
                return Err(invalid(format!(
                    "rule '{}' message expression calls a user-defined func; \
                     helper predicates are not usable in messages",
                    rule.name
                )));
            }
        }
    }
    Ok(())
}

fn invalid(reason: String) -> ReaperError {
    ReaperError::InvalidPolicy { reason }
}

fn too_deep(limit: usize) -> ReaperError {
    ReaperError::InvalidPolicy {
        reason: format!(
            "policy nesting depth exceeds the maximum of {limit} after function \
             inlining; deeply nested conditions/expressions are rejected to keep \
             evaluation total and terminating (set REAPER_MAX_NESTING_DEPTH to adjust)"
        ),
    }
}

pub(crate) fn qualified(f: &FuncDef) -> String {
    match &f.namespace {
        Some(ns) => format!("{ns}::{}", f.name),
        None => f.name.clone(),
    }
}

/// Entity keywords that root attribute accesses (`user.role`, `input.x.y`).
pub(crate) const ENTITY_KEYWORDS: &[&str] = &["user", "actor", "resource", "context", "input"];

/// Request pseudo-variables the AST evaluator binds in every scope (bare
/// entity ids usable as function arguments, e.g. `rebac::related(user, ...)`).
const PSEUDO_VARS: &[&str] = &["user", "resource", "actor"];

/// Is `name` an entity reference rather than a variable? Covers both bare
/// entity keywords and the parser's dotted pseudo-variable form for entity
/// method calls (`Variable("user.email")`).
fn is_entity_rooted_name(name: &str) -> bool {
    ENTITY_KEYWORDS.contains(&name)
        || ENTITY_KEYWORDS
            .iter()
            .any(|e| name.starts_with(e) && name.as_bytes().get(e.len()) == Some(&b'.'))
}

/// Collect variable names a condition tree REFERENCES (reads), excluding
/// entity-keyword-rooted accesses (those are entity references).
fn collect_referenced_vars_condition(cond: &Condition, out: &mut HashSet<String>) {
    fn from_expr(e: &Expr, out: &mut HashSet<String>) {
        match e {
            Expr::Variable(v) => {
                out.insert(v.clone());
            }
            Expr::AttributeAccess { variable, .. } | Expr::IndexedAccess { variable, .. } => {
                out.insert(variable.clone());
            }
            Expr::MethodCall { receiver, args, .. } => {
                from_expr(receiver, out);
                for a in args {
                    from_expr(a, out);
                }
            }
            Expr::FunctionCall { args, .. } => {
                for a in args {
                    from_expr(a, out);
                }
            }
            Expr::Literal(_) => {}
        }
    }
    fn from_left(l: &ComparisonLeft, out: &mut HashSet<String>) {
        match l {
            ComparisonLeft::VarAttr(va) => {
                out.insert(va.variable.clone());
            }
            ComparisonLeft::Expr(e) => from_expr(e, out),
            ComparisonLeft::EntityAttr(_) => {}
        }
    }
    fn from_right(r: &ComparisonRight, out: &mut HashSet<String>) {
        match r {
            ComparisonRight::Variable(v) => {
                out.insert(v.clone());
            }
            ComparisonRight::VarAttr(va) => {
                out.insert(va.variable.clone());
            }
            ComparisonRight::Expr(e) => from_expr(e, out),
            ComparisonRight::Value(_) | ComparisonRight::EntityAttr(_) => {}
        }
    }
    fn from_assignment(v: &AssignmentValue, out: &mut HashSet<String>) {
        match v {
            AssignmentValue::Variable(name) => {
                out.insert(name.clone());
            }
            AssignmentValue::Expr(e) => from_expr(e, out),
            AssignmentValue::Comparison { left, right, .. } => {
                from_left(left, out);
                from_right(right, out);
            }
            AssignmentValue::Comprehension(comp) => {
                let (outputs, iterator, filters): (Vec<&Expr>, _, &Vec<Condition>) = match comp {
                    Comprehension::Set {
                        output,
                        iterator,
                        filters,
                    }
                    | Comprehension::Array {
                        output,
                        iterator,
                        filters,
                    } => (vec![output], iterator, filters),
                    Comprehension::Object {
                        key,
                        value,
                        iterator,
                        filters,
                    } => (vec![key, value], iterator, filters),
                };
                match &iterator.collection {
                    super::ast::IterationSource::VarAttr(va) => {
                        out.insert(va.variable.clone());
                    }
                    super::ast::IterationSource::IndexedVariable { variable, .. } => {
                        out.insert(variable.clone());
                    }
                    super::ast::IterationSource::EntityAttr(_) => {}
                }
                for o in outputs {
                    from_expr(o, out);
                }
                for f in filters {
                    collect_referenced_vars_condition(f, out);
                }
            }
            AssignmentValue::EntityAttr(_) | AssignmentValue::Value(_) => {}
        }
    }
    match cond {
        Condition::Comparison { left, right, .. } => {
            from_left(left, out);
            from_right(right, out);
        }
        Condition::Assignment { value, .. } => from_assignment(value, out),
        Condition::And(cs) | Condition::Or(cs) => {
            for c in cs {
                collect_referenced_vars_condition(c, out);
            }
        }
        Condition::Not(inner) => collect_referenced_vars_condition(inner, out),
        Condition::Expr(e) => from_expr(e, out),
        Condition::True | Condition::False => {}
    }
}

/// Collect variable names a condition tree BINDS (assignments and
/// comprehension iterator variables).
pub(crate) fn collect_bound_vars_condition(cond: &Condition, out: &mut HashSet<String>) {
    match cond {
        Condition::Assignment { variable, value } => {
            out.insert(variable.clone());
            if let AssignmentValue::Comprehension(c) = value {
                collect_bound_vars_comprehension(c, out);
            }
        }
        Condition::And(cs) | Condition::Or(cs) => {
            for c in cs {
                collect_bound_vars_condition(c, out);
            }
        }
        Condition::Not(inner) => collect_bound_vars_condition(inner, out),
        Condition::True | Condition::False | Condition::Comparison { .. } | Condition::Expr(_) => {}
    }
}

fn collect_bound_vars_comprehension(comp: &Comprehension, out: &mut HashSet<String>) {
    let (iterator, filters) = match comp {
        Comprehension::Set {
            iterator, filters, ..
        }
        | Comprehension::Array {
            iterator, filters, ..
        }
        | Comprehension::Object {
            iterator, filters, ..
        } => (iterator, filters),
    };
    out.insert(iterator.variable.clone());
    for f in filters {
        collect_bound_vars_condition(f, out);
    }
}

// ---------------------------------------------------------------------------
// Measuring walk: structural max depth + user-function call sites, with
// unknown-namespace and arity validation at each call. Self-limiting: errors
// at limit+1 and unwinds, same posture as `limits::check_policy_depth`.
// ---------------------------------------------------------------------------

struct MeasureCtx<'a> {
    limit: usize,
    functions: &'a [FuncDef],
    /// Import aliases declared but possibly not yet resolved; calls into
    /// these namespaces are deferred instead of rejected.
    declared_aliases: HashSet<&'a str>,
}

impl MeasureCtx<'_> {
    fn measure_condition(
        &self,
        cond: &Condition,
        depth: usize,
        sites: &mut Vec<(usize, usize)>,
    ) -> Result<usize, ReaperError> {
        if depth > self.limit {
            return Err(too_deep(self.limit));
        }
        let mut max = depth;
        match cond {
            Condition::True | Condition::False => {}
            Condition::Comparison { left, right, .. } => {
                if let ComparisonLeft::Expr(e) = left {
                    max = max.max(self.measure_expr_at(e, depth + 2, sites)?);
                }
                if let ComparisonRight::Expr(e) = right {
                    max = max.max(self.measure_expr_at(e, depth + 2, sites)?);
                }
                max = max.max(depth + 1);
            }
            Condition::Assignment { value, .. } => {
                max = max.max(self.measure_assignment(value, depth + 1, sites)?);
            }
            Condition::And(cs) | Condition::Or(cs) => {
                for c in cs {
                    max = max.max(self.measure_condition(c, depth + 1, sites)?);
                }
            }
            Condition::Not(inner) => {
                max = max.max(self.measure_condition(inner, depth + 1, sites)?);
            }
            Condition::Expr(e) => {
                max = max.max(self.measure_expr_at(e, depth + 1, sites)?);
            }
        }
        Ok(max)
    }

    fn measure_assignment(
        &self,
        value: &AssignmentValue,
        depth: usize,
        sites: &mut Vec<(usize, usize)>,
    ) -> Result<usize, ReaperError> {
        if depth > self.limit {
            return Err(too_deep(self.limit));
        }
        let mut max = depth;
        match value {
            AssignmentValue::Expr(e) => {
                max = max.max(self.measure_expr_at(e, depth + 1, sites)?);
            }
            AssignmentValue::Comparison { left, right, .. } => {
                if let ComparisonLeft::Expr(e) = left {
                    max = max.max(self.measure_expr_at(e, depth + 2, sites)?);
                }
                if let ComparisonRight::Expr(e) = right {
                    max = max.max(self.measure_expr_at(e, depth + 2, sites)?);
                }
                max = max.max(depth + 1);
            }
            AssignmentValue::Comprehension(comp) => {
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
                    max = max.max(self.measure_expr_at(out, depth + 1, sites)?);
                }
                for f in filters {
                    max = max.max(self.measure_condition(f, depth + 1, sites)?);
                }
            }
            AssignmentValue::EntityAttr(_)
            | AssignmentValue::Value(_)
            | AssignmentValue::Variable(_) => {}
        }
        Ok(max)
    }

    /// Public-ish entry used for rule messages (starts at the given depth).
    fn measure_expr(
        &self,
        expr: &Expr,
        depth: usize,
        sites: &mut Vec<(usize, usize)>,
    ) -> Result<usize, ReaperError> {
        self.measure_expr_at(expr, depth, sites)
    }

    fn measure_expr_at(
        &self,
        expr: &Expr,
        depth: usize,
        sites: &mut Vec<(usize, usize)>,
    ) -> Result<usize, ReaperError> {
        if depth > self.limit {
            return Err(too_deep(self.limit));
        }
        let mut max = depth;
        match expr {
            Expr::MethodCall { receiver, args, .. } => {
                max = max.max(self.measure_expr_at(receiver, depth + 1, sites)?);
                for a in args {
                    max = max.max(self.measure_expr_at(a, depth + 1, sites)?);
                }
            }
            Expr::FunctionCall {
                namespace,
                function,
                args,
            } => {
                match find_function(self.functions, namespace.as_deref(), function) {
                    Some(i) => {
                        let expected = self.functions[i].params.len();
                        if args.len() != expected {
                            return Err(invalid(format!(
                                "func '{}' takes {expected} argument{}, called with {}",
                                qualified(&self.functions[i]),
                                if expected == 1 { "" } else { "s" },
                                args.len()
                            )));
                        }
                        sites.push((depth, i));
                    }
                    None => {
                        // A namespaced call must target a builtin namespace,
                        // a defined function, or a DECLARED import alias
                        // (whose functions merge at load time; the load path
                        // re-validates post-merge with no deferral).
                        // Un-namespaced unknown names keep their historical
                        // runtime behavior (builtin dispatch errors at eval).
                        if let Some(ns) = namespace.as_deref() {
                            if !BUILTIN_NAMESPACES.contains(&ns)
                                && !self.declared_aliases.contains(ns)
                            {
                                if self
                                    .functions
                                    .iter()
                                    .any(|f| f.namespace.as_deref() == Some(ns))
                                {
                                    return Err(invalid(format!(
                                        "namespace '{ns}' has no function named '{function}'"
                                    )));
                                }
                                return Err(invalid(format!(
                                    "call to '{ns}::{function}' references unknown \
                                     namespace '{ns}' (not a builtin namespace, a \
                                     defined function's namespace, or a declared \
                                     import alias)"
                                )));
                            }
                        }
                    }
                }
                for a in args {
                    max = max.max(self.measure_expr_at(a, depth + 1, sites)?);
                }
            }
            Expr::Literal(_)
            | Expr::Variable(_)
            | Expr::AttributeAccess { .. }
            | Expr::IndexedAccess { .. } => {}
        }
        Ok(max)
    }
}
