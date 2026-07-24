//! Compile-pass function inlining (R4-01 Phase C, ADR-2).
//!
//! The compiled path never grows a call stack: user-defined `func` calls are
//! expanded IN PLACE before rule compilation, so evaluation stays a flat
//! condition walk with zero new runtime machinery. Totality (call-graph DAG +
//! depth cap, enforced by `reap::functions`) makes the expansion always
//! terminate, and the nesting cap is re-applied to the expanded tree.
//!
//! ## Equivalence contract
//!
//! The AST interpreter implements `func` calls call-by-value (arguments
//! evaluate once, eagerly, in the caller's scope; the body runs in a fresh
//! scope of parameters only). Substitution is only equivalent to that when
//! re-evaluating an argument expression cannot be OBSERVED — so arguments are
//! restricted to pure, infallible, stable shapes:
//!
//! - **literals** — trivially stable;
//! - **entity/input paths** (`user.role`, `input.request.kind`) — reads are
//!   total (missing ⇒ null) and stable within one evaluation;
//! - **previously bound variables** (`x := ... && f(x)`) — a bound-variable
//!   read never errors and the binding cannot change between the call site
//!   and the expanded body.
//!
//! Anything else (method/function-call arguments, unbound variables) is
//! rejected — the rule keeps its per-rule AST fallback (observable, never
//! silent, R4-01 A.2), where call-by-value is implemented directly.
//!
//! Two further guards keep the expansion capture-free:
//! - function bodies have **no free variables** (validated at parse), so an
//!   expanded body can never accidentally read a caller rule variable;
//! - body-local bindings are **alpha-renamed** per call site (`__f<n>_x`),
//!   which cannot collide with author variables (identifiers must start with
//!   a letter).

use super::super::ast::{
    AssignmentValue, ComparisonLeft, ComparisonRight, Comprehension, ComprehensionIterator,
    Condition, Entity, EntityAttr, Expr, FuncDef, Index, IterationSource, Policy, Rule, VarAttr,
};
use crate::reap::functions::{collect_bound_vars_condition, find_function, qualified};
use reaper_core::ReaperError;
use std::collections::{HashMap, HashSet};

/// Expand every user-func call in every rule. Policies without functions pass
/// through untouched (and pay nothing). The returned policy carries the
/// expanded rules and an empty function list.
pub(super) fn inline_policy(policy: Policy) -> Result<Policy, ReaperError> {
    if policy.functions.is_empty() {
        return Ok(policy);
    }
    let mut inliner = Inliner {
        functions: &policy.functions,
        gensym: 0,
    };
    let mut rules = Vec::with_capacity(policy.rules.len());
    for rule in &policy.rules {
        let mut bound = HashSet::new();
        let condition = inliner.inline_condition(&rule.condition, &mut bound)?;
        rules.push(Rule {
            name: rule.name.clone(),
            decision: rule.decision.clone(),
            condition,
            message: rule.message.clone(),
        });
    }
    let inlined = Policy {
        name: policy.name,
        metadata: policy.metadata,
        default_decision: policy.default_decision,
        rules,
        functions: Vec::new(),
        imports: policy.imports,
    };
    // ADR-2: the nesting cap applies to the POST-inline tree. The analysis in
    // `reap::functions` already bounds inline-effective depth, so this is the
    // belt-and-suspenders re-check against the actual expansion.
    crate::reap::limits::check_policy_depth(
        &inlined,
        crate::reap::limits::configured_max_nesting_depth(),
    )?;
    Ok(inlined)
}

/// How one parameter substitutes into the body.
enum ArgSubst {
    /// Argument was a bound variable: rename the parameter to it.
    Rename(String),
    /// Argument was a literal or entity/input path: splice the expression,
    /// canonicalized per position.
    Splice(Expr),
}

struct Inliner<'a> {
    functions: &'a [FuncDef],
    gensym: usize,
}

impl Inliner<'_> {
    /// Inline user-func calls in condition position, threading the set of
    /// variables BOUND so far in evaluation order (And chains accumulate;
    /// Or/Not branch bindings don't escape — mirroring the runtime's
    /// short-circuit order, same discipline as `entity_var_compares_dominated`).
    fn inline_condition(
        &mut self,
        cond: &Condition,
        bound: &mut HashSet<String>,
    ) -> Result<Condition, ReaperError> {
        match cond {
            Condition::True => Ok(Condition::True),
            Condition::False => Ok(Condition::False),
            Condition::And(cs) => {
                let mut out = Vec::with_capacity(cs.len());
                for c in cs {
                    let inlined = self.inline_condition(c, bound)?;
                    collect_bound_vars_condition(&inlined, bound);
                    out.push(inlined);
                }
                Ok(Condition::And(out))
            }
            Condition::Or(cs) => {
                let mut out = Vec::with_capacity(cs.len());
                for c in cs {
                    // Branch bindings don't escape the branch.
                    let mut branch_bound = bound.clone();
                    out.push(self.inline_condition(c, &mut branch_bound)?);
                }
                Ok(Condition::Or(out))
            }
            Condition::Not(inner) => {
                let mut branch_bound = bound.clone();
                Ok(Condition::Not(Box::new(
                    self.inline_condition(inner, &mut branch_bound)?,
                )))
            }
            Condition::Expr(e) => {
                if let Expr::FunctionCall {
                    namespace,
                    function,
                    args,
                } = e
                {
                    if let Some(i) = find_function(self.functions, namespace.as_deref(), function) {
                        return self.expand_call(i, args, bound);
                    }
                }
                reject_nested_user_calls(e, self.functions)?;
                Ok(cond.clone())
            }
            Condition::Comparison { left, op, right } => {
                if let ComparisonLeft::Expr(e) = left {
                    reject_nested_user_calls(e, self.functions)?;
                }
                if let ComparisonRight::Expr(e) = right {
                    reject_nested_user_calls(e, self.functions)?;
                }
                Ok(Condition::Comparison {
                    left: left.clone(),
                    op: *op,
                    right: right.clone(),
                })
            }
            Condition::Assignment { variable, value } => {
                let value = match value {
                    AssignmentValue::Comprehension(comp) => {
                        AssignmentValue::Comprehension(self.inline_comprehension(comp, bound)?)
                    }
                    AssignmentValue::Expr(e) => {
                        reject_nested_user_calls(e, self.functions)?;
                        value.clone()
                    }
                    AssignmentValue::Comparison { left, right, .. } => {
                        if let ComparisonLeft::Expr(e) = left {
                            reject_nested_user_calls(e, self.functions)?;
                        }
                        if let ComparisonRight::Expr(e) = right {
                            reject_nested_user_calls(e, self.functions)?;
                        }
                        value.clone()
                    }
                    AssignmentValue::EntityAttr(_)
                    | AssignmentValue::Value(_)
                    | AssignmentValue::Variable(_) => value.clone(),
                };
                bound.insert(variable.clone());
                Ok(Condition::Assignment {
                    variable: variable.clone(),
                    value,
                })
            }
        }
    }

    /// Comprehension FILTERS are condition positions — calls in them inline.
    /// The iterator variable is bound within the filters.
    fn inline_comprehension(
        &mut self,
        comp: &Comprehension,
        bound: &HashSet<String>,
    ) -> Result<Comprehension, ReaperError> {
        let inline_filters = |this: &mut Self,
                              iterator: &ComprehensionIterator,
                              filters: &[Condition]|
         -> Result<Vec<Condition>, ReaperError> {
            let mut fbound = bound.clone();
            fbound.insert(iterator.variable.clone());
            let mut out = Vec::with_capacity(filters.len());
            for f in filters {
                let inlined = this.inline_condition(f, &mut fbound)?;
                collect_bound_vars_condition(&inlined, &mut fbound);
                out.push(inlined);
            }
            Ok(out)
        };
        Ok(match comp {
            Comprehension::Set {
                output,
                iterator,
                filters,
            } => {
                reject_nested_user_calls(output, self.functions)?;
                Comprehension::Set {
                    output: output.clone(),
                    iterator: iterator.clone(),
                    filters: inline_filters(self, iterator, filters)?,
                }
            }
            Comprehension::Array {
                output,
                iterator,
                filters,
            } => {
                reject_nested_user_calls(output, self.functions)?;
                Comprehension::Array {
                    output: output.clone(),
                    iterator: iterator.clone(),
                    filters: inline_filters(self, iterator, filters)?,
                }
            }
            Comprehension::Object {
                key,
                value,
                iterator,
                filters,
            } => {
                reject_nested_user_calls(key, self.functions)?;
                reject_nested_user_calls(value, self.functions)?;
                Comprehension::Object {
                    key: key.clone(),
                    value: value.clone(),
                    iterator: iterator.clone(),
                    filters: inline_filters(self, iterator, filters)?,
                }
            }
        })
    }

    /// Expand one call: validate argument shapes, alpha-rename body-local
    /// bindings, substitute parameters, then recursively inline the result
    /// (nested calls in the body expand with the substituted arguments).
    fn expand_call(
        &mut self,
        index: usize,
        args: &[Expr],
        bound: &mut HashSet<String>,
    ) -> Result<Condition, ReaperError> {
        let def = &self.functions[index];
        if args.len() != def.params.len() {
            return Err(err(format!(
                "func '{}' takes {} arguments, called with {}",
                qualified(def),
                def.params.len(),
                args.len()
            )));
        }

        let mut subst: HashMap<String, ArgSubst> = HashMap::with_capacity(args.len() + 4);
        for (p, a) in def.params.iter().zip(args) {
            let s = match a {
                Expr::Variable(v) => {
                    if bound.contains(v) {
                        ArgSubst::Rename(v.clone())
                    } else {
                        return Err(err(format!(
                            "func '{}' argument '{v}' is not bound before the call; \
                             only literals, entity/input paths, and previously \
                             assigned variables compile as arguments — the rule \
                             runs on the AST evaluator",
                            qualified(def)
                        )));
                    }
                }
                Expr::Literal(_) => ArgSubst::Splice(a.clone()),
                Expr::AttributeAccess { variable, .. }
                    if crate::reap::functions::ENTITY_KEYWORDS.contains(&variable.as_str()) =>
                {
                    ArgSubst::Splice(a.clone())
                }
                Expr::IndexedAccess {
                    variable, index, ..
                } if crate::reap::functions::ENTITY_KEYWORDS.contains(&variable.as_str()) => {
                    if matches!(index, Index::Wildcard) {
                        return Err(err(format!(
                            "func '{}' argument uses a wildcard index; wildcard \
                             arguments are not compiled — the rule runs on the \
                             AST evaluator",
                            qualified(def)
                        )));
                    }
                    ArgSubst::Splice(a.clone())
                }
                other => {
                    return Err(err(format!(
                        "func '{}' argument {other:?} is not compiled (only literals, \
                         entity/input paths, and previously assigned variables); \
                         the rule runs on the AST evaluator",
                        qualified(def)
                    )));
                }
            };
            subst.insert(p.clone(), s);
        }

        // Alpha-rename body-local bindings per call site. Cannot collide with
        // params (shadowing rejected at parse) or author variables (authors
        // cannot write leading underscores).
        let mut body_bound = HashSet::new();
        collect_bound_vars_condition(&def.body, &mut body_bound);
        let call_id = self.gensym;
        self.gensym += 1;
        for name in body_bound {
            subst.insert(
                name.clone(),
                ArgSubst::Rename(format!("__f{call_id}_{name}")),
            );
        }

        let substituted = subst_condition(&def.body, &subst)?;

        // Nested calls in the body now carry substituted arguments (renamed
        // variables stay "bound": renamed bindings enter `body_scope` as the
        // walk crosses them; spliced paths/literals need no binding).
        let mut body_scope = bound.clone();
        self.inline_condition(&substituted, &mut body_scope)
    }
}

fn err(reason: String) -> ReaperError {
    ReaperError::InvalidPolicy { reason }
}

/// A user-func call anywhere but condition position (assignment values,
/// builtin arguments, comparison sides) is not inlined — reject so the rule
/// takes its per-rule AST fallback, where the interpreter handles it.
fn reject_nested_user_calls(expr: &Expr, functions: &[FuncDef]) -> Result<(), ReaperError> {
    match expr {
        Expr::FunctionCall {
            namespace,
            function,
            args,
        } => {
            if find_function(functions, namespace.as_deref(), function).is_some() {
                return Err(err(format!(
                    "call to func '{}{}' outside condition position is not \
                     compiled; the rule runs on the AST evaluator",
                    namespace
                        .as_deref()
                        .map(|ns| format!("{ns}::"))
                        .unwrap_or_default(),
                    function
                )));
            }
            for a in args {
                reject_nested_user_calls(a, functions)?;
            }
            Ok(())
        }
        Expr::MethodCall { receiver, args, .. } => {
            reject_nested_user_calls(receiver, functions)?;
            for a in args {
                reject_nested_user_calls(a, functions)?;
            }
            Ok(())
        }
        Expr::Literal(_)
        | Expr::Variable(_)
        | Expr::AttributeAccess { .. }
        | Expr::IndexedAccess { .. } => Ok(()),
    }
}

// ---------------------------------------------------------------------------
// Substitution: replace parameter references with arguments, canonicalizing
// to the AST shapes the parser would have produced for hand-written source —
// so the expanded tree compiles with the existing machinery.
// ---------------------------------------------------------------------------

fn subst_condition(
    cond: &Condition,
    subst: &HashMap<String, ArgSubst>,
) -> Result<Condition, ReaperError> {
    Ok(match cond {
        Condition::True => Condition::True,
        Condition::False => Condition::False,
        Condition::And(cs) => Condition::And(
            cs.iter()
                .map(|c| subst_condition(c, subst))
                .collect::<Result<_, _>>()?,
        ),
        Condition::Or(cs) => Condition::Or(
            cs.iter()
                .map(|c| subst_condition(c, subst))
                .collect::<Result<_, _>>()?,
        ),
        Condition::Not(inner) => Condition::Not(Box::new(subst_condition(inner, subst)?)),
        Condition::Expr(e) => Condition::Expr(subst_expr(e, subst)?),
        Condition::Comparison { left, op, right } => Condition::Comparison {
            left: subst_comparison_left(left, subst)?,
            op: *op,
            right: subst_comparison_right(right, subst)?,
        },
        Condition::Assignment { variable, value } => Condition::Assignment {
            variable: rename_if_mapped(variable, subst)?,
            value: subst_assignment(value, subst)?,
        },
    })
}

/// Binding positions (assignment targets, iterator variables) may only be
/// RENAMED, never spliced — parameters cannot be assigned (shadowing is
/// rejected at parse), so a Splice here is unreachable for validated ASTs.
fn rename_if_mapped(name: &str, subst: &HashMap<String, ArgSubst>) -> Result<String, ReaperError> {
    match subst.get(name) {
        None => Ok(name.to_string()),
        Some(ArgSubst::Rename(to)) => Ok(to.clone()),
        Some(ArgSubst::Splice(_)) => Err(err(format!(
            "func body assigns to parameter '{name}'; not compiled — the rule \
             runs on the AST evaluator"
        ))),
    }
}

fn subst_expr(expr: &Expr, subst: &HashMap<String, ArgSubst>) -> Result<Expr, ReaperError> {
    Ok(match expr {
        Expr::Literal(_) => expr.clone(),
        Expr::Variable(v) => match subst.get(v) {
            None => expr.clone(),
            Some(ArgSubst::Rename(to)) => Expr::Variable(to.clone()),
            Some(ArgSubst::Splice(arg)) => arg.clone(),
        },
        Expr::AttributeAccess {
            variable,
            attribute,
        } => match subst.get(variable) {
            None => expr.clone(),
            Some(ArgSubst::Rename(to)) => Expr::AttributeAccess {
                variable: to.clone(),
                attribute: attribute.clone(),
            },
            Some(ArgSubst::Splice(arg)) => {
                let (root, path) = input_path_join(arg, attribute)?;
                Expr::AttributeAccess {
                    variable: root,
                    attribute: path,
                }
            }
        },
        Expr::IndexedAccess {
            variable,
            attribute,
            index,
        } => match subst.get(variable) {
            None => expr.clone(),
            Some(ArgSubst::Rename(to)) => Expr::IndexedAccess {
                variable: to.clone(),
                attribute: attribute.clone(),
                index: index.clone(),
            },
            Some(ArgSubst::Splice(arg)) => {
                let (root, path) = input_path_join(arg, attribute)?;
                Expr::IndexedAccess {
                    variable: root,
                    attribute: path,
                    index: index.clone(),
                }
            }
        },
        Expr::MethodCall {
            receiver,
            method,
            args,
        } => Expr::MethodCall {
            receiver: Box::new(subst_expr(receiver, subst)?),
            method: method.clone(),
            args: args
                .iter()
                .map(|a| subst_expr(a, subst))
                .collect::<Result<_, _>>()?,
        },
        Expr::FunctionCall {
            namespace,
            function,
            args,
        } => Expr::FunctionCall {
            namespace: namespace.clone(),
            function: function.clone(),
            args: args
                .iter()
                .map(|a| subst_expr(a, subst))
                .collect::<Result<_, _>>()?,
        },
    })
}

/// Join a spliced argument with a trailing attribute path (`p.attr` where
/// `p := input.request` becomes `input.request.attr`). Only `input`-rooted,
/// un-indexed arguments join: input navigation is dotted-path based on both
/// evaluators (missing ⇒ null), so the join preserves semantics. Entity
/// attribute stores are flat — a joined path would silently read a different
/// (missing) key — so those reject to the AST fallback instead.
fn input_path_join(arg: &Expr, attribute: &str) -> Result<(String, String), ReaperError> {
    match arg {
        Expr::AttributeAccess {
            variable,
            attribute: arg_attr,
        } if variable == "input" => {
            let path = if attribute.is_empty() {
                arg_attr.clone()
            } else {
                format!("{arg_attr}.{attribute}")
            };
            Ok((variable.clone(), path))
        }
        _ => Err(err(format!(
            "func parameter is dereferenced (`.{attribute}`) but its argument \
             {arg:?} is not an input path; not compiled — the rule runs on the \
             AST evaluator"
        ))),
    }
}

/// Convert a spliced argument to an `EntityAttr` when it is entity-rooted.
fn expr_to_entity_attr(arg: &Expr) -> Option<EntityAttr> {
    match arg {
        Expr::AttributeAccess {
            variable,
            attribute,
        } if crate::reap::functions::ENTITY_KEYWORDS.contains(&variable.as_str()) => {
            Some(EntityAttr {
                entity: Entity::from(variable.as_str()),
                attribute: attribute.clone(),
                index: None,
            })
        }
        Expr::IndexedAccess {
            variable,
            attribute,
            index,
        } if crate::reap::functions::ENTITY_KEYWORDS.contains(&variable.as_str()) => {
            Some(EntityAttr {
                entity: Entity::from(variable.as_str()),
                attribute: attribute.clone(),
                index: Some(index.clone()),
            })
        }
        _ => None,
    }
}

fn subst_comparison_left(
    left: &ComparisonLeft,
    subst: &HashMap<String, ArgSubst>,
) -> Result<ComparisonLeft, ReaperError> {
    Ok(match left {
        ComparisonLeft::EntityAttr(_) => left.clone(),
        ComparisonLeft::VarAttr(va) => match subst.get(&va.variable) {
            None => left.clone(),
            Some(ArgSubst::Rename(to)) => ComparisonLeft::VarAttr(VarAttr {
                variable: to.clone(),
                attribute: va.attribute.clone(),
                index: va.index.clone(),
            }),
            Some(ArgSubst::Splice(arg)) => {
                let (_, path) = input_path_join(arg, &va.attribute)?;
                ComparisonLeft::EntityAttr(EntityAttr {
                    entity: Entity::Input,
                    attribute: path,
                    index: va.index.clone(),
                })
            }
        },
        ComparisonLeft::Expr(e) => {
            // A bare parameter on the left (`p == "x"`) canonicalizes to the
            // shape the parser would emit for the argument written in place.
            if let Expr::Variable(v) = e {
                match subst.get(v) {
                    Some(ArgSubst::Splice(arg)) => {
                        if let Some(ea) = expr_to_entity_attr(arg) {
                            return Ok(ComparisonLeft::EntityAttr(ea));
                        }
                        return Ok(ComparisonLeft::Expr(arg.clone()));
                    }
                    Some(ArgSubst::Rename(to)) => {
                        return Ok(ComparisonLeft::Expr(Expr::Variable(to.clone())));
                    }
                    None => {}
                }
            }
            ComparisonLeft::Expr(subst_expr(e, subst)?)
        }
    })
}

fn subst_comparison_right(
    right: &ComparisonRight,
    subst: &HashMap<String, ArgSubst>,
) -> Result<ComparisonRight, ReaperError> {
    Ok(match right {
        ComparisonRight::Value(_) | ComparisonRight::EntityAttr(_) => right.clone(),
        ComparisonRight::Variable(v) => match subst.get(v) {
            None => right.clone(),
            Some(ArgSubst::Rename(to)) => ComparisonRight::Variable(to.clone()),
            Some(ArgSubst::Splice(arg)) => {
                if let Expr::Literal(val) = arg {
                    ComparisonRight::Value(val.clone())
                } else if let Some(ea) = expr_to_entity_attr(arg) {
                    ComparisonRight::EntityAttr(ea)
                } else {
                    ComparisonRight::Expr(arg.clone())
                }
            }
        },
        ComparisonRight::VarAttr(va) => match subst.get(&va.variable) {
            None => right.clone(),
            Some(ArgSubst::Rename(to)) => ComparisonRight::VarAttr(VarAttr {
                variable: to.clone(),
                attribute: va.attribute.clone(),
                index: va.index.clone(),
            }),
            Some(ArgSubst::Splice(arg)) => {
                let (_, path) = input_path_join(arg, &va.attribute)?;
                ComparisonRight::EntityAttr(EntityAttr {
                    entity: Entity::Input,
                    attribute: path,
                    index: va.index.clone(),
                })
            }
        },
        ComparisonRight::Expr(e) => ComparisonRight::Expr(subst_expr(e, subst)?),
    })
}

fn subst_assignment(
    value: &AssignmentValue,
    subst: &HashMap<String, ArgSubst>,
) -> Result<AssignmentValue, ReaperError> {
    Ok(match value {
        AssignmentValue::EntityAttr(_) | AssignmentValue::Value(_) => value.clone(),
        AssignmentValue::Variable(v) => match subst.get(v) {
            None => value.clone(),
            Some(ArgSubst::Rename(to)) => AssignmentValue::Variable(to.clone()),
            Some(ArgSubst::Splice(arg)) => {
                if let Expr::Literal(val) = arg {
                    AssignmentValue::Value(val.clone())
                } else if let Some(ea) = expr_to_entity_attr(arg) {
                    AssignmentValue::EntityAttr(ea)
                } else {
                    AssignmentValue::Expr(arg.clone())
                }
            }
        },
        AssignmentValue::Expr(e) => AssignmentValue::Expr(subst_expr(e, subst)?),
        AssignmentValue::Comparison { left, op, right } => AssignmentValue::Comparison {
            left: subst_comparison_left(left, subst)?,
            op: *op,
            right: subst_comparison_right(right, subst)?,
        },
        AssignmentValue::Comprehension(comp) => {
            AssignmentValue::Comprehension(subst_comprehension(comp, subst)?)
        }
    })
}

fn subst_comprehension(
    comp: &Comprehension,
    subst: &HashMap<String, ArgSubst>,
) -> Result<Comprehension, ReaperError> {
    let subst_iterator = |iterator: &ComprehensionIterator| -> Result<_, ReaperError> {
        let collection = match &iterator.collection {
            IterationSource::EntityAttr(_) => iterator.collection.clone(),
            IterationSource::VarAttr(va) => match subst.get(&va.variable) {
                None => iterator.collection.clone(),
                Some(ArgSubst::Rename(to)) => IterationSource::VarAttr(VarAttr {
                    variable: to.clone(),
                    attribute: va.attribute.clone(),
                    index: va.index.clone(),
                }),
                Some(ArgSubst::Splice(arg)) => {
                    let (_, path) = input_path_join(arg, &va.attribute)?;
                    IterationSource::EntityAttr(EntityAttr {
                        entity: Entity::Input,
                        attribute: path,
                        index: va.index.clone(),
                    })
                }
            },
            IterationSource::IndexedVariable { variable, index } => match subst.get(variable) {
                None => iterator.collection.clone(),
                Some(ArgSubst::Rename(to)) => IterationSource::IndexedVariable {
                    variable: to.clone(),
                    index: index.clone(),
                },
                Some(ArgSubst::Splice(arg)) => {
                    // Iterating a spliced path (`x := p[_]`): any entity-
                    // rooted collection works — this is exactly the shape the
                    // parser emits for `entity.attr[_]`.
                    match expr_to_entity_attr(arg) {
                        Some(mut ea) if ea.index.is_none() => {
                            ea.index = Some(index.clone());
                            IterationSource::EntityAttr(ea)
                        }
                        _ => {
                            return Err(err(format!(
                                "func body iterates parameter '{variable}' but its \
                                 argument {arg:?} is not an un-indexed entity/input \
                                 path; not compiled — the rule runs on the AST \
                                 evaluator"
                            )))
                        }
                    }
                }
            },
        };
        Ok(ComprehensionIterator {
            variable: rename_if_mapped(&iterator.variable, subst)?,
            collection,
        })
    };
    Ok(match comp {
        Comprehension::Set {
            output,
            iterator,
            filters,
        } => Comprehension::Set {
            output: Box::new(subst_expr(output, subst)?),
            iterator: subst_iterator(iterator)?,
            filters: filters
                .iter()
                .map(|f| subst_condition(f, subst))
                .collect::<Result<_, _>>()?,
        },
        Comprehension::Array {
            output,
            iterator,
            filters,
        } => Comprehension::Array {
            output: Box::new(subst_expr(output, subst)?),
            iterator: subst_iterator(iterator)?,
            filters: filters
                .iter()
                .map(|f| subst_condition(f, subst))
                .collect::<Result<_, _>>()?,
        },
        Comprehension::Object {
            key,
            value,
            iterator,
            filters,
        } => Comprehension::Object {
            key: Box::new(subst_expr(key, subst)?),
            value: Box::new(subst_expr(value, subst)?),
            iterator: subst_iterator(iterator)?,
            filters: filters
                .iter()
                .map(|f| subst_condition(f, subst))
                .collect::<Result<_, _>>()?,
        },
    })
}
