//! Compiler: AST → ReaperDSLEvaluator
//!
//! Transforms parsed .reap AST into optimized ReaperDSLEvaluator for sub-microsecond evaluation.
//!
//! ## Module Structure
//! - `helpers`: Extraction and conversion utilities
//! - `comprehension`: Comprehension compilation
//! - `methods`: Method call compilation
//! - `expression`: Expression type compilation
//! - `comparison`: Comparison compilation

mod comparison;
mod comprehension;
mod expression;
mod helpers;
mod methods;

use comparison::{compile_comparison, compile_comparison_assignment};
use comprehension::{compile_comprehension_output, compile_iterator};
use expression::compile_expression_assignment;
use helpers::{extract_entity_attr, extract_int_literal, extract_string_literal};
use methods::compile_method_call;

use super::ast::{
    AssignmentValue, Comprehension, Condition, Decision, Entity, Expr, Index, Policy, Rule,
};
use crate::evaluators::reaper_dsl::{
    Condition as DslCondition, EntityType as DslEntityType, ExprType,
    LiteralValue as DslLiteralValue, Message as DslMessage, MessagePart as DslMessagePart,
    NumericOp, ReaperDSLEvaluator, Rule as DslRule, TimeCondition, UncompiledComprehensionType,
};
use crate::{data::DataStore, PolicyAction};
use reaper_core::ReaperError;
use std::sync::Arc;

/// Compile a parsed policy AST into a ReaperDSLEvaluator
pub fn compile_policy(
    policy: Policy,
    store: Arc<DataStore>,
) -> Result<ReaperDSLEvaluator, ReaperError> {
    // Bound structural nesting before the recursive compile walk. Covers ASTs
    // that reach the compiler without going through the pest parser's pre-scan
    // (the YAML/JSON policy formats, or a directly-constructed AST) — Plan 05,
    // Step 2.
    crate::reap::limits::enforce_policy_depth(&policy)?;

    // Convert default decision
    let default_decision = match policy.default_decision {
        Decision::Allow => PolicyAction::Allow,
        Decision::Deny => PolicyAction::Deny,
    };

    // Compile rules
    let mut rules = Vec::new();
    for rule in policy.rules {
        rules.push(compile_rule(rule)?);
    }

    let evaluator = ReaperDSLEvaluator::new(store, rules, default_decision);
    Ok(evaluator)
}

/// Compile a single rule
fn compile_rule(rule: Rule) -> Result<DslRule, ReaperError> {
    let decision = match rule.decision {
        Decision::Allow => PolicyAction::Allow,
        Decision::Deny => PolicyAction::Deny,
    };

    // R4-01 A.3: `entity.attr == <var>` may lower to the compiled
    // `EqualsVariable` shape only when every such use is dominated by its
    // binding (see `entity_var_compares_dominated`) — decided once per rule.
    let allow_var_compare = entity_var_compares_dominated(&rule.condition);
    // Check-mode message (R4-01 B.3): lower `with message <expr>` to
    // literal/variable parts. An expression shape the lowering does not
    // cover keeps the RULE on the AST evaluator (per-rule fallback) so
    // check-mode output stays byte-identical.
    let message = rule.message.as_ref().map(lower_message).transpose()?;
    // Message variables must be assignment targets somewhere in the rule.
    // The compiled renderer mirrors the interpreter's full resolution chain
    // (rule variables → request context → "Undefined variable" error), so
    // this guard is a conservative compile-surface fence, not a correctness
    // requirement: a message naming a variable the rule never assigns is an
    // unusual shape (context-map reads, typos) that stays on the AST
    // evaluator where behavior is definitionally right.
    if let Some(msg) = &message {
        let mut assigned = std::collections::HashSet::new();
        collect_assigned_vars(&rule.condition, &mut assigned);
        let msg_vars: Vec<&String> = match msg {
            DslMessage::Literal(_) => Vec::new(),
            DslMessage::Variable(v) => vec![v],
            DslMessage::Concat(parts) => parts
                .iter()
                .filter_map(|p| match p {
                    DslMessagePart::Variable(v) => Some(v),
                    DslMessagePart::Literal(_) => None,
                })
                .collect(),
        };
        for v in msg_vars {
            if !assigned.contains(v.as_str()) {
                return Err(ReaperError::InvalidPolicy {
                    reason: format!(
                        "check-mode message references variable '{v}' that the rule \
                         never binds; the rule runs on the AST evaluator"
                    ),
                });
            }
        }
    }
    let condition = compile_condition_with(rule.condition, allow_var_compare)?;

    Ok(DslRule {
        name: rule.name,
        condition,
        decision,
        message,
    })
}

/// Collect every variable name the condition tree ASSIGNS (any
/// `AssignmentValue` form), for the message-variable guard above.
fn collect_assigned_vars(cond: &Condition, out: &mut std::collections::HashSet<String>) {
    match cond {
        Condition::Assignment { variable, .. } => {
            out.insert(variable.clone());
        }
        Condition::And(children) | Condition::Or(children) => {
            for c in children {
                collect_assigned_vars(c, out);
            }
        }
        Condition::Not(inner) => collect_assigned_vars(inner, out),
        Condition::True | Condition::False | Condition::Comparison { .. } | Condition::Expr(_) => {}
    }
}

/// Lower a check-mode message expression to renderable parts: string
/// literals, rule-variable references, and `concat(...)` of those.
fn lower_message(expr: &Expr) -> Result<DslMessage, ReaperError> {
    match expr {
        Expr::Literal(super::ast::Value::String(s)) => Ok(DslMessage::Literal(s.clone())),
        Expr::Variable(v) => Ok(DslMessage::Variable(v.clone())),
        Expr::FunctionCall {
            namespace: None,
            function,
            args,
        } if function == "concat" => {
            // Arguments must be string literals or variables — nested
            // concat calls do NOT lower. Flattening a nested concat would
            // reorder the interpreter's error sequence (an inner concat's
            // type check runs during OUTER argument evaluation, before
            // later outer arguments evaluate), breaking byte-identical
            // error parity. Nested concat keeps the rule on the AST
            // evaluator.
            let mut parts = Vec::with_capacity(args.len());
            for arg in args {
                parts.push(match arg {
                    Expr::Literal(super::ast::Value::String(s)) => {
                        DslMessagePart::Literal(s.clone())
                    }
                    Expr::Variable(v) => DslMessagePart::Variable(v.clone()),
                    other => {
                        return Err(ReaperError::InvalidPolicy {
                            reason: format!(
                                "check-mode concat argument {other:?} is not compiled \
                                 (only string literals and variables); the rule runs \
                                 on the AST evaluator"
                            ),
                        })
                    }
                });
            }
            Ok(DslMessage::Concat(parts))
        }
        other => Err(ReaperError::InvalidPolicy {
            reason: format!(
                "check-mode message expression {other:?} is not compiled (only string \
                 literals, variables, and concat of those); the rule runs on the AST \
                 evaluator"
            ),
        }),
    }
}

/// R4-01 A.3 soundness guard: may `entity.attr ==/!= <var>` comparisons in
/// this rule lower to the compiled `EqualsVariable` shape?
///
/// The compiled shape reads an UNBOUND variable as a non-match (`false`)
/// while the interpreter reports an evaluation error — so the lowering is
/// permitted only when evaluation order guarantees the variable is bound at
/// every such use, making the divergence unobservable. Conservative
/// dominance walk, mirroring the runtime's left-to-right short-circuit:
/// `And` chains accumulate bindings in order; bindings made inside `Or`/
/// `Not` branches never escape the branch (the branch may be skipped at
/// runtime). Any un-dominated use ⇒ `false` ⇒ the rule keeps its AST
/// fallback (cheap and observable since per-rule fallback, A.2).
fn entity_var_compares_dominated(cond: &Condition) -> bool {
    use crate::reap::ast::ComparisonRight;
    fn walk(cond: &Condition, bound: &mut std::collections::HashSet<String>) -> bool {
        match cond {
            Condition::Assignment { variable, .. } => {
                bound.insert(variable.clone());
                true
            }
            Condition::Comparison {
                right: ComparisonRight::Variable(v),
                ..
            } => bound.contains(v),
            Condition::Comparison { .. } => true,
            Condition::And(children) => children.iter().all(|c| walk(c, bound)),
            Condition::Or(children) => children.iter().all(|c| {
                let mut branch = bound.clone();
                walk(c, &mut branch)
            }),
            Condition::Not(inner) => {
                let mut branch = bound.clone();
                walk(inner, &mut branch)
            }
            Condition::True | Condition::False | Condition::Expr(_) => true,
        }
    }
    walk(cond, &mut std::collections::HashSet::new())
}

/// Compile a condition expression (public shim kept for existing callers:
/// variable-compare lowering disabled, the conservative default).
fn compile_condition(cond: Condition) -> Result<DslCondition, ReaperError> {
    compile_condition_with(cond, false)
}

/// Compile a condition expression. `allow_var_compare` is the per-rule
/// dominance verdict from [`entity_var_compares_dominated`].
fn compile_condition_with(
    cond: Condition,
    allow_var_compare: bool,
) -> Result<DslCondition, ReaperError> {
    match cond {
        Condition::True => Ok(DslCondition::Always),

        Condition::False => {
            // False condition = Not(Always)
            Ok(DslCondition::Not(Box::new(DslCondition::Always)))
        }

        Condition::Comparison { left, op, right } => {
            compile_comparison(left, op, right, allow_var_compare)
        }

        Condition::And(conditions) => {
            let mut compiled = Vec::new();
            for c in conditions {
                compiled.push(compile_condition_with(c, allow_var_compare)?);
            }
            Ok(DslCondition::And(compiled))
        }

        Condition::Or(conditions) => {
            let mut compiled = Vec::new();
            for c in conditions {
                compiled.push(compile_condition_with(c, allow_var_compare)?);
            }
            Ok(DslCondition::Or(compiled))
        }

        Condition::Not(cond) => {
            let compiled = compile_condition_with(*cond, allow_var_compare)?;
            Ok(DslCondition::Not(Box::new(compiled)))
        }

        Condition::Assignment { variable, value } => {
            match value {
                // Comprehension assignment: arr := [x | x := user.items[_]; filter]
                AssignmentValue::Comprehension(comp) => {
                    compile_comprehension_assignment(variable, comp)
                }

                // Expression assignment: x := user.name.lower()
                AssignmentValue::Expr(expr) => compile_expression_assignment(variable, expr),

                // Entity attribute assignment: x := user.role
                AssignmentValue::EntityAttr(attr) => {
                    // `x := input.<dotted.path>` (R4-01 B.3): lower to an
                    // input-read expression assignment; the path pre-parses
                    // at compile and missing doc/path binds Null (assignment
                    // still succeeds), matching the interpreter's total
                    // input access. Indexed input reads stay on the AST
                    // evaluator (the wildcard form is B.2's iteration
                    // source, not an assignment value).
                    if matches!(attr.entity, Entity::Input) {
                        if attr.index.is_some() {
                            return Err(ReaperError::InvalidPolicy {
                                reason: "indexed `input` assignment is not compiled; the \
                                         rule runs on the AST evaluator"
                                    .to_string(),
                            });
                        }
                        return Ok(DslCondition::ExpressionAssignment {
                            variable,
                            expr_type: ExprType::InputRead {
                                path: attr.attribute,
                            },
                        });
                    }
                    let entity_type = match attr.entity {
                        Entity::User => DslEntityType::User,
                        Entity::Resource => DslEntityType::Resource,
                        Entity::Context => DslEntityType::Context,
                        Entity::Actor => DslEntityType::Actor,
                        Entity::Input => unreachable!("Input entity handled above"),
                    };
                    let index = attr.index.map(|i| match i {
                        Index::Number(n) => crate::evaluators::reaper_dsl::IndexExpr::Number(n),
                        Index::String(s) => crate::evaluators::reaper_dsl::IndexExpr::String(s),
                        Index::Wildcard => crate::evaluators::reaper_dsl::IndexExpr::Wildcard,
                    });
                    Ok(DslCondition::Assignment {
                        variable,
                        entity_type,
                        attribute: attr.attribute,
                        index,
                    })
                }

                // Variable reference: x := y
                AssignmentValue::Variable(var_ref) => {
                    // Variable-to-variable assignment: use ExpressionAssignment with VariableRef
                    Ok(DslCondition::ExpressionAssignment {
                        variable,
                        expr_type: ExprType::VariableRef { variable: var_ref },
                    })
                }

                // Literal value assignment: x := "admin" (R4-01 A.3). Scalar
                // string/int/bool literals lower to a constant-load
                // expression assignment; float/null/composite literals stay
                // on the AST evaluator (CompiledLiteralValue has no such
                // variants — widening it is a later slice, tracked in
                // REGO_GAP_ANALYSIS §4).
                AssignmentValue::Value(val) => {
                    use super::ast::Value;
                    let literal = match val {
                        Value::String(s) => DslLiteralValue::String(s),
                        Value::Integer(n) => DslLiteralValue::Int(n),
                        Value::Boolean(b) => DslLiteralValue::Bool(b),
                        other => {
                            return Err(ReaperError::InvalidPolicy {
                                reason: format!(
                                    "Literal assignment of {other:?} is not supported in \
                                     compiled policies (only string/int/bool literals \
                                     compile); the policy runs on the AST evaluator."
                                ),
                            })
                        }
                    };
                    Ok(DslCondition::ExpressionAssignment {
                        variable,
                        expr_type: ExprType::Literal { value: literal },
                    })
                }

                // Comparison assignment: x := user.age >= 18
                AssignmentValue::Comparison { left, op, right } => {
                    compile_comparison_assignment(variable, left, op, right)
                }
            }
        }

        Condition::Expr(expr) => {
            // Compile expression-based conditions (function calls, method calls)
            compile_expr_condition(expr)
        }
    }
}

/// Compile an expression into a DslCondition
/// Supports function calls (regex::matches, time::is_after, etc.) and method calls (.contains, .startswith, etc.)
fn compile_expr_condition(expr: Expr) -> Result<DslCondition, ReaperError> {
    match expr {
        Expr::FunctionCall {
            namespace,
            function,
            args,
        } => compile_function_call(namespace, function, args),

        Expr::MethodCall {
            receiver,
            method,
            args,
        } => compile_method_call(*receiver, method, args),

        // Variable as standalone condition: checks if variable is truthy (true for bool, non-null for others)
        Expr::Variable(var_name) => Ok(DslCondition::VariableIsTruthy { variable: var_name }),

        _ => Err(ReaperError::InvalidPolicy {
            reason: format!(
                "Expression type {:?} is not supported as a standalone condition. \
                Only function calls (regex::matches, time::is_after) and method calls \
                (.contains, .startswith, .endswith) are supported.",
                expr
            ),
        }),
    }
}

/// Compile a function call expression (e.g., regex::matches(user.email, "pattern"))
fn compile_function_call(
    namespace: Option<String>,
    function: String,
    args: Vec<Expr>,
) -> Result<DslCondition, ReaperError> {
    let ns = namespace.as_deref().unwrap_or("");

    match (ns, function.as_str()) {
        // rebac::related / reachable / inherited — compiled to interned graph
        // lookups. Subject/object must be `user`/`resource` or literals here;
        // dynamic (variable) ids run on the AST evaluator instead.
        ("rebac", "related") | ("rebac", "reachable") | ("rebac", "inherited") => {
            compile_rebac_call(&function, args)
        }

        // taint::trusted("key") — provenance gate (F1 agentic authz). Only a
        // literal key compiles; a dynamic key expression runs on the AST
        // evaluator instead.
        ("taint", "trusted") => {
            if args.len() != 1 {
                return Err(ReaperError::InvalidPolicy {
                    reason: format!("taint::trusted requires 1 argument, got {}", args.len()),
                });
            }
            let key = extract_string_literal(&args[0])?;
            Ok(DslCondition::TaintTrusted { key })
        }

        // regex::matches(entity.attribute, "pattern")
        ("regex", "matches") => {
            if args.len() != 2 {
                return Err(ReaperError::InvalidPolicy {
                    reason: format!(
                        "regex::matches requires 2 arguments (attribute, pattern), got {}",
                        args.len()
                    ),
                });
            }

            let (entity_type, attribute) = extract_entity_attr(&args[0])?;
            let pattern = extract_string_literal(&args[1])?;

            // Validate regex pattern at compile time
            if regex::Regex::new(&pattern).is_err() {
                return Err(ReaperError::InvalidPolicy {
                    reason: format!("Invalid regex pattern: {}", pattern),
                });
            }

            Ok(DslCondition::RegexMatches {
                entity_type,
                attribute,
                pattern,
            })
        }

        // time::is_after(entity.attribute, threshold)
        ("time", "is_after") => {
            if args.len() != 2 {
                return Err(ReaperError::InvalidPolicy {
                    reason: format!(
                        "time::is_after requires 2 arguments (attribute, threshold), got {}",
                        args.len()
                    ),
                });
            }

            let (entity_type, attribute) = extract_entity_attr(&args[0])?;
            let threshold = extract_int_literal(&args[1])?;

            Ok(DslCondition::TimeOp(TimeCondition {
                entity_type,
                attribute,
                op: NumericOp::Greater, // IsAfter = Greater
                threshold,
            }))
        }

        // time::is_before(entity.attribute, threshold)
        ("time", "is_before") => {
            if args.len() != 2 {
                return Err(ReaperError::InvalidPolicy {
                    reason: format!(
                        "time::is_before requires 2 arguments (attribute, threshold), got {}",
                        args.len()
                    ),
                });
            }

            let (entity_type, attribute) = extract_entity_attr(&args[0])?;
            let threshold = extract_int_literal(&args[1])?;

            Ok(DslCondition::TimeOp(TimeCondition {
                entity_type,
                attribute,
                op: NumericOp::Less, // IsBefore = Less
                threshold,
            }))
        }

        // Type check functions: is_string, is_number, is_bool
        ("", "is_string") => {
            if args.len() != 1 {
                return Err(ReaperError::InvalidPolicy {
                    reason: format!("is_string requires 1 argument, got {}", args.len()),
                });
            }
            // Check if argument is a variable (no dot) or entity.attribute
            if let Expr::Variable(var_name) = &args[0] {
                if !var_name.contains('.') {
                    // It's a simple variable
                    return Ok(DslCondition::VariableIsString {
                        variable: var_name.clone(),
                    });
                }
            }
            let (entity_type, attribute) = extract_entity_attr(&args[0])?;
            Ok(DslCondition::IsString {
                entity_type,
                attribute,
            })
        }

        ("", "is_number") => {
            if args.len() != 1 {
                return Err(ReaperError::InvalidPolicy {
                    reason: format!("is_number requires 1 argument, got {}", args.len()),
                });
            }
            // Check if argument is a variable (no dot) or entity.attribute
            if let Expr::Variable(var_name) = &args[0] {
                if !var_name.contains('.') {
                    // It's a simple variable
                    return Ok(DslCondition::VariableIsNumber {
                        variable: var_name.clone(),
                    });
                }
            }
            let (entity_type, attribute) = extract_entity_attr(&args[0])?;
            Ok(DslCondition::IsNumber {
                entity_type,
                attribute,
            })
        }

        ("", "is_bool") | ("", "is_boolean") => {
            if args.len() != 1 {
                return Err(ReaperError::InvalidPolicy {
                    reason: format!("is_bool requires 1 argument, got {}", args.len()),
                });
            }
            // Check if argument is a variable (no dot) or entity.attribute
            if let Expr::Variable(var_name) = &args[0] {
                if !var_name.contains('.') {
                    // It's a simple variable
                    return Ok(DslCondition::VariableIsBool {
                        variable: var_name.clone(),
                    });
                }
            }
            let (entity_type, attribute) = extract_entity_attr(&args[0])?;
            Ok(DslCondition::IsBool {
                entity_type,
                attribute,
            })
        }

        _ => {
            let fn_prefix = if ns.is_empty() {
                String::new()
            } else {
                format!("{}::", ns)
            };
            Err(ReaperError::InvalidPolicy {
                reason: format!(
                    "Unsupported function call: {}{}. Supported functions: \
                    regex::matches, time::is_after, time::is_before, is_string, is_number, is_bool",
                    fn_prefix, function
                ),
            })
        }
    }
}

/// Compile a comprehension assignment: arr := [x | x := user.items[_]; filter]
fn compile_comprehension_assignment(
    variable: String,
    comp: Comprehension,
) -> Result<DslCondition, ReaperError> {
    match comp {
        Comprehension::Array {
            output,
            iterator,
            filters,
        } => {
            let compiled_output = compile_comprehension_output(&output)?;
            let (iter_var, iter_source) = compile_iterator(iterator)?;
            let compiled_filters = filters
                .into_iter()
                .map(compile_condition)
                .collect::<Result<Vec<_>, _>>()?;

            Ok(DslCondition::ComprehensionAssignment {
                variable,
                comp_type: UncompiledComprehensionType::Array,
                iterator_var: iter_var,
                iterator_source: iter_source,
                filters: compiled_filters,
                output: Some(compiled_output),
                key_output: None,
            })
        }

        Comprehension::Set {
            output,
            iterator,
            filters,
        } => {
            let compiled_output = compile_comprehension_output(&output)?;
            let (iter_var, iter_source) = compile_iterator(iterator)?;
            let compiled_filters = filters
                .into_iter()
                .map(compile_condition)
                .collect::<Result<Vec<_>, _>>()?;

            Ok(DslCondition::ComprehensionAssignment {
                variable,
                comp_type: UncompiledComprehensionType::Set,
                iterator_var: iter_var,
                iterator_source: iter_source,
                filters: compiled_filters,
                output: Some(compiled_output),
                key_output: None,
            })
        }

        Comprehension::Object {
            key,
            value,
            iterator,
            filters,
        } => {
            let compiled_key = compile_comprehension_output(&key)?;
            let compiled_value = compile_comprehension_output(&value)?;
            let (iter_var, iter_source) = compile_iterator(iterator)?;
            let compiled_filters = filters
                .into_iter()
                .map(compile_condition)
                .collect::<Result<Vec<_>, _>>()?;

            Ok(DslCondition::ComprehensionAssignment {
                variable,
                comp_type: UncompiledComprehensionType::Object,
                iterator_var: iter_var,
                iterator_source: iter_source,
                filters: compiled_filters,
                output: Some(compiled_value),
                key_output: Some(compiled_key),
            })
        }
    }
}

/// Lower a rebac::* function call into a RebacCheck condition. Only static
/// argument shapes compile (the sub-microsecond path needs ids resolvable
/// without variable state); anything else errors, which routes the policy to
/// the AST evaluator.
fn compile_rebac_call(function: &str, args: Vec<Expr>) -> Result<DslCondition, ReaperError> {
    use crate::evaluators::reaper_dsl::{RebacKind, RebacRef};

    let kind = match function {
        "related" => RebacKind::Direct,
        "reachable" => RebacKind::Reachable,
        _ => RebacKind::Inherited,
    };
    let expected = if kind == RebacKind::Direct { 3 } else { 5 };
    if args.len() != expected {
        return Err(ReaperError::InvalidPolicy {
            reason: format!(
                "rebac::{function} requires {expected} arguments, got {}",
                args.len()
            ),
        });
    }

    let rebac_ref = |expr: &Expr| -> Result<RebacRef, ReaperError> {
        match expr {
            Expr::Variable(name) if name == "user" => Ok(RebacRef::Principal),
            Expr::Variable(name) if name == "resource" => Ok(RebacRef::ResourceId),
            // F1 agentic authz: the optional non-human actor. Evaluates as
            // non-matching when the request carries no actor (the AST
            // evaluator rejects the unbound pseudo-variable the same way).
            Expr::Variable(name) if name == "actor" => Ok(RebacRef::Actor),
            Expr::Literal(crate::reap::ast::Value::String(s)) => Ok(RebacRef::Literal(s.clone())),
            other => Err(ReaperError::InvalidPolicy {
                reason: format!(
                    "rebac::{function}: only `user`, `actor`, `resource`, or string literals compile (got {other:?}); dynamic ids use the AST evaluator"
                ),
            }),
        }
    };
    let literal_str = |expr: &Expr, what: &str| -> Result<String, ReaperError> {
        match expr {
            Expr::Literal(crate::reap::ast::Value::String(s)) => Ok(s.clone()),
            other => Err(ReaperError::InvalidPolicy {
                reason: format!(
                    "rebac::{function}: {what} must be a string literal, got {other:?}"
                ),
            }),
        }
    };

    let subject = rebac_ref(&args[0])?;
    let relation = literal_str(&args[1], "relation")?;
    let object = rebac_ref(&args[2])?;
    let (via, max_depth) = if kind == RebacKind::Direct {
        (None, 1)
    } else {
        let via = literal_str(&args[3], "edge")?;
        let max = match &args[4] {
            Expr::Literal(crate::reap::ast::Value::Integer(n)) if *n > 0 => (*n as u32).min(16),
            other => {
                return Err(ReaperError::InvalidPolicy {
                    reason: format!(
                        "rebac::{function}: max_depth must be a positive integer literal, got {other:?}"
                    ),
                })
            }
        };
        (Some(via), max)
    };

    Ok(DslCondition::RebacCheck {
        kind,
        subject,
        relation,
        object,
        via,
        max_depth,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evaluators::PolicyEvaluator;
    use crate::reap::ast::{ComparisonLeft, ComparisonRight, EntityAttr, Operator, Value};
    use crate::PolicyRequest;
    use crate::{data::DataStore, EntityBuilder};
    use std::collections::HashMap;

    #[test]
    fn test_compile_simple_rule() {
        let policy = Policy {
            name: "test".to_string(),
            metadata: HashMap::new(),
            default_decision: Decision::Deny,
            rules: vec![Rule {
                message: None,
                name: "admin".to_string(),
                decision: Decision::Allow,
                condition: Condition::Comparison {
                    left: ComparisonLeft::EntityAttr(EntityAttr {
                        entity: Entity::User,
                        attribute: "role".to_string(),
                        index: None,
                    }),
                    op: Operator::Equal,
                    right: ComparisonRight::Value(Value::String("admin".to_string())),
                },
            }],
        };

        let store = Arc::new(DataStore::new());
        let evaluator = compile_policy(policy, store.clone()).unwrap();

        // Create test entities
        let interner = store.interner();
        let alice_id = interner.intern("alice");
        let user_type = interner.intern("User");
        let role_key = interner.intern("role");
        let admin_value = interner.intern("admin");

        let alice = EntityBuilder::new(alice_id, user_type)
            .with_string(role_key, admin_value)
            .build();

        let doc_id = interner.intern("doc1");
        let doc_type = interner.intern("Document");
        let doc = EntityBuilder::new(doc_id, doc_type).build();

        store.insert(alice);
        store.insert(doc);

        // Evaluate
        let mut context = HashMap::new();
        context.insert("principal".to_string(), "alice".to_string());

        let request = PolicyRequest {
            resource: "doc1".to_string(),
            action: "read".to_string(),
            context,

            ..Default::default()
        };

        let decision = evaluator.evaluate(&request).unwrap();
        assert!(matches!(decision, PolicyAction::Allow));
    }

    #[test]
    fn test_compile_expression_assignment() {
        // Test parsing and compiling a policy with expression assignment
        let policy_text = r#"
policy test_expr {
    version: "1.0",
    default: deny,

    rule lowercase_match {
        allow if {
            user.name != null &&
            resource.type == "test" &&
            lower_name := user.name.lower() &&
            lower_name == "admin"
        }
    }
}
"#;
        let parsed = crate::reap::ReapParser::parse(policy_text).expect("Parse failed");
        let store = Arc::new(DataStore::new());
        let interner = store.interner();

        // Create user with mixed case name
        let user_id = interner.intern("user1");
        let user_type = interner.intern("User");
        let name_key = interner.intern("name");
        let name_value = interner.intern("ADMIN");

        let user = EntityBuilder::new(user_id, user_type)
            .with_string(name_key, name_value)
            .build();

        // Create resource
        let res_id = interner.intern("res1");
        let res_type = interner.intern("Resource");
        let type_key = interner.intern("type");
        let type_value = interner.intern("test");

        let resource = EntityBuilder::new(res_id, res_type)
            .with_string(type_key, type_value)
            .build();

        store.insert(user);
        store.insert(resource);

        // Compile and evaluate
        let evaluator = compile_policy(parsed, store).expect("Compile failed");

        let mut context = HashMap::new();
        context.insert("principal".to_string(), "user1".to_string());

        let request = PolicyRequest {
            resource: "res1".to_string(),
            action: "read".to_string(),
            context,

            ..Default::default()
        };

        let decision = evaluator.evaluate(&request).expect("Evaluation failed");
        assert!(
            matches!(decision, PolicyAction::Allow),
            "Expected Allow but got {:?}",
            decision
        );
    }

    #[test]
    fn test_compile_context_entity() {
        // Test parsing and compiling a policy with context entity access
        let policy_text = r#"
policy test_context {
    version: "1.0",
    default: deny,

    rule context_check {
        allow if {
            context.env == "production"
        }
    }
}
"#;
        let parsed = crate::reap::ReapParser::parse(policy_text).expect("Parse failed");
        let store = Arc::new(DataStore::new());
        let interner = store.interner();

        // Create user (minimal)
        let user_id = interner.intern("user1");
        let user_type = interner.intern("User");
        let user = EntityBuilder::new(user_id, user_type).build();
        store.insert(user);

        // Create resource
        let res_id = interner.intern("res1");
        let res_type = interner.intern("Resource");
        let resource = EntityBuilder::new(res_id, res_type).build();
        store.insert(resource);

        // Compile
        let evaluator = compile_policy(parsed, store).expect("Compile failed");

        // Test with matching context
        let mut context = HashMap::new();
        context.insert("principal".to_string(), "user1".to_string());
        context.insert("env".to_string(), "production".to_string());

        let request = PolicyRequest {
            resource: "res1".to_string(),
            action: "read".to_string(),
            context,

            ..Default::default()
        };

        let decision = evaluator.evaluate(&request).expect("Evaluation failed");
        assert!(
            matches!(decision, PolicyAction::Allow),
            "Expected Allow but got {:?}",
            decision
        );

        // Test with non-matching context
        let mut context2 = HashMap::new();
        context2.insert("principal".to_string(), "user1".to_string());
        context2.insert("env".to_string(), "development".to_string());

        let request2 = PolicyRequest {
            resource: "res1".to_string(),
            action: "read".to_string(),
            context: context2,

            ..Default::default()
        };

        let decision2 = evaluator.evaluate(&request2).expect("Evaluation failed");
        assert!(
            matches!(decision2, PolicyAction::Deny),
            "Expected Deny but got {:?}",
            decision2
        );
    }

    #[test]
    fn test_compile_comprehension_assignment() {
        // Test parsing and compiling a policy with comprehension assignment
        let policy_text = r#"
policy test_comp {
    version: "1.0",
    default: deny,

    rule skills_check {
        allow if {
            user.skills != null &&
            resource.type == "test" &&
            all_skills := [s | s := user.skills[_]] &&
            all_skills.count() >= 2
        }
    }
}
"#;
        let parsed = crate::reap::ReapParser::parse(policy_text).expect("Parse failed");
        let store = Arc::new(DataStore::new());
        let interner = store.interner();

        // Create user with skills array
        let user_id = interner.intern("user1");
        let user_type = interner.intern("User");
        let skills_key = interner.intern("skills");

        use crate::data::AttributeValue;
        let skill1 = interner.intern("rust");
        let skill2 = interner.intern("python");
        let skill3 = interner.intern("go");
        let skills = AttributeValue::List(vec![
            AttributeValue::String(skill1),
            AttributeValue::String(skill2),
            AttributeValue::String(skill3),
        ]);

        let mut attrs = HashMap::new();
        attrs.insert(skills_key, skills);
        let user = crate::data::Entity::new(user_id, user_type, attrs);
        store.insert(user);

        // Create resource
        let res_id = interner.intern("res1");
        let res_type = interner.intern("Resource");
        let type_key = interner.intern("type");
        let type_value = interner.intern("test");

        let resource = EntityBuilder::new(res_id, res_type)
            .with_string(type_key, type_value)
            .build();
        store.insert(resource);

        // Compile and evaluate
        let evaluator = compile_policy(parsed, store).expect("Compile failed");

        let mut context = HashMap::new();
        context.insert("principal".to_string(), "user1".to_string());

        let request = PolicyRequest {
            resource: "res1".to_string(),
            action: "read".to_string(),
            context,

            ..Default::default()
        };

        let decision = evaluator.evaluate(&request).expect("Evaluation failed");
        assert!(
            matches!(decision, PolicyAction::Allow),
            "Expected Allow but got {:?}",
            decision
        );
    }
}
