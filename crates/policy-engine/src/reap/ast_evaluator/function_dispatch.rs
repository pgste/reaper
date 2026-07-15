//! Function call dispatch for AST evaluator.
//!
//! This module handles dispatching namespace function calls:
//! - Type checking: is_string, is_number, is_bool, is_array, is_set, is_object, is_null
//! - String: concat
//! - Time namespace: now_ns, now_ms, now, parse_rfc3339, format_rfc3339, add_ns, subtract_ns, is_before, is_after, is_between
//! - Math namespace: abs, round, floor, ceil, sqrt, pow, min, max, clamp
//! - Regex namespace: is_valid, escape, matches, replace, split
//! - JSON namespace: parse, stringify, is_valid

use super::builtin_functions;
use super::types::{EvalContext, EvalValue};
use super::ReapAstEvaluator;
use crate::reap::ast::Expr;
use reaper_core::ReaperError;

impl ReapAstEvaluator {
    /// Evaluate function call expressions (e.g., time.now_ns(), concat(a, b))
    pub(super) fn evaluate_function_call(
        &self,
        namespace: Option<&str>,
        function: &str,
        args: &[Expr],
        context: &EvalContext,
    ) -> Result<EvalValue, ReaperError> {
        match (namespace, function) {
            // Type checking functions (using builtin_functions)
            (None, "is_string") => {
                if args.is_empty() {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "is_string() requires one argument".to_string(),
                    });
                }
                let value = self.evaluate_expr(&args[0], context)?;
                Ok(builtin_functions::is_string(&value))
            }
            (None, "is_number") => {
                if args.is_empty() {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "is_number() requires one argument".to_string(),
                    });
                }
                let value = self.evaluate_expr(&args[0], context)?;
                Ok(builtin_functions::is_number(&value))
            }
            (None, "is_bool") => {
                if args.is_empty() {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "is_bool() requires one argument".to_string(),
                    });
                }
                let value = self.evaluate_expr(&args[0], context)?;
                Ok(builtin_functions::is_bool(&value))
            }
            (None, "is_array") => {
                if args.is_empty() {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "is_array() requires one argument".to_string(),
                    });
                }
                let value = self.evaluate_expr(&args[0], context)?;
                Ok(builtin_functions::is_array(&value))
            }
            (None, "is_set") => {
                if args.is_empty() {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "is_set() requires one argument".to_string(),
                    });
                }
                let value = self.evaluate_expr(&args[0], context)?;
                Ok(builtin_functions::is_set(&value))
            }
            (None, "is_object") => {
                if args.is_empty() {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "is_object() requires one argument".to_string(),
                    });
                }
                let value = self.evaluate_expr(&args[0], context)?;
                Ok(builtin_functions::is_object(&value))
            }
            (None, "is_null") => {
                if args.is_empty() {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "is_null() requires one argument".to_string(),
                    });
                }
                let value = self.evaluate_expr(&args[0], context)?;
                Ok(builtin_functions::is_null(&value))
            }

            // String concatenation (using builtin_functions)
            (None, "concat") => {
                let values: Result<Vec<EvalValue>, _> = args
                    .iter()
                    .map(|arg| self.evaluate_expr(arg, context))
                    .collect();
                builtin_functions::concat(&values?)
            }

            // ===== Time/Date Functions (using builtin_functions::time) =====
            (Some("time"), "now_ns") => {
                if !args.is_empty() {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "time::now_ns() takes no arguments".to_string(),
                    });
                }
                builtin_functions::time_now_ns()
            }

            (Some("time"), "now_ms") => {
                if !args.is_empty() {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "time::now_ms() takes no arguments".to_string(),
                    });
                }
                builtin_functions::time_now_ms()
            }

            (Some("time"), "now") => {
                if !args.is_empty() {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "time::now() takes no arguments".to_string(),
                    });
                }
                builtin_functions::time_now()
            }

            (Some("time"), "parse_rfc3339") => {
                if args.len() != 1 {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "time::parse_rfc3339() requires exactly one argument (string)"
                            .to_string(),
                    });
                }
                let value = self.evaluate_expr(&args[0], context)?;
                builtin_functions::time_parse_rfc3339(&value)
            }

            (Some("time"), "format_rfc3339") => {
                if args.len() != 1 {
                    return Err(ReaperError::InvalidPolicy {
                        reason:
                            "time::format_rfc3339() requires exactly one argument (nanoseconds)"
                                .to_string(),
                    });
                }
                let value = self.evaluate_expr(&args[0], context)?;
                builtin_functions::time_format_rfc3339(&value)
            }

            (Some("time"), "add_ns") => {
                if args.len() != 2 {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "time::add_ns() requires exactly two arguments (timestamp_ns, duration_ns)".to_string(),
                    });
                }
                let timestamp = self.evaluate_expr(&args[0], context)?;
                let duration = self.evaluate_expr(&args[1], context)?;
                builtin_functions::time_add_ns(&timestamp, &duration)
            }

            (Some("time"), "subtract_ns") => {
                if args.len() != 2 {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "time::subtract_ns() requires exactly two arguments (timestamp_ns, duration_ns)".to_string(),
                    });
                }
                let timestamp = self.evaluate_expr(&args[0], context)?;
                let duration = self.evaluate_expr(&args[1], context)?;
                builtin_functions::time_subtract_ns(&timestamp, &duration)
            }

            (Some("time"), "is_before") => {
                if args.len() != 2 {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "time::is_before() requires exactly two arguments (t1, t2)"
                            .to_string(),
                    });
                }
                let t1 = self.evaluate_expr(&args[0], context)?;
                let t2 = self.evaluate_expr(&args[1], context)?;
                builtin_functions::time_is_before(&t1, &t2)
            }

            (Some("time"), "is_after") => {
                if args.len() != 2 {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "time::is_after() requires exactly two arguments (t1, t2)"
                            .to_string(),
                    });
                }
                let t1 = self.evaluate_expr(&args[0], context)?;
                let t2 = self.evaluate_expr(&args[1], context)?;
                builtin_functions::time_is_after(&t1, &t2)
            }

            (Some("time"), "is_between") => {
                if args.len() != 3 {
                    return Err(ReaperError::InvalidPolicy {
                        reason:
                            "time::is_between() requires exactly three arguments (t, start, end)"
                                .to_string(),
                    });
                }
                let t = self.evaluate_expr(&args[0], context)?;
                let start = self.evaluate_expr(&args[1], context)?;
                let end = self.evaluate_expr(&args[2], context)?;
                builtin_functions::time_is_between(&t, &start, &end)
            }

            // Regex namespace functions (escape uses builtin_functions, others use cache)
            (Some("regex"), "is_valid") => {
                if args.len() != 1 {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "regex::is_valid() requires exactly one argument (pattern)"
                            .to_string(),
                    });
                }
                let pattern = match self.evaluate_expr(&args[0], context)? {
                    EvalValue::String(s) => s,
                    _ => {
                        return Err(ReaperError::InvalidPolicy {
                            reason: "regex::is_valid() argument must be a string".to_string(),
                        })
                    }
                };
                // Use cached regex compilation - if valid, it gets cached for future use
                let is_valid = self.get_cached_regex(&pattern).is_ok();
                Ok(EvalValue::Boolean(is_valid))
            }

            (Some("regex"), "escape") => {
                if args.len() != 1 {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "regex::escape() requires exactly one argument (string)"
                            .to_string(),
                    });
                }
                let value = self.evaluate_expr(&args[0], context)?;
                builtin_functions::regex_escape(&value)
            }

            // ===== JWT / auth artifacts =====
            // jwt::decode(token) -> claims object (NO signature verification;
            // verify at the trust boundary — OPA io.jwt.decode parity).
            (Some("jwt"), "decode") => {
                if args.len() != 1 {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "jwt::decode(token) takes exactly 1 argument".to_string(),
                    });
                }
                let token = self.rebac_string_arg(&args[0], context, "jwt::decode", "token")?;
                builtin_functions::jwt::decode(&token)
            }
            // jwt::header(token) -> {alg, kid, typ, ...}
            (Some("jwt"), "header") => {
                if args.len() != 1 {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "jwt::header(token) takes exactly 1 argument".to_string(),
                    });
                }
                let token = self.rebac_string_arg(&args[0], context, "jwt::header", "token")?;
                builtin_functions::jwt::header(&token)
            }
            (Some("time"), "now_secs") => {
                if !args.is_empty() {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "time::now_secs() takes no arguments".to_string(),
                    });
                }
                builtin_functions::time::now_secs()
            }

            // ===== ReBAC (relationship graph) =====
            // rebac::related(subject, relation, object) -> bool
            (Some("rebac"), "related") => {
                let Some((subject, relation, object)) =
                    self.rebac_ids_3(args, context, "related")?
                else {
                    // Unbound actor argument: the check is false, never an error.
                    return Ok(EvalValue::Boolean(false));
                };
                Ok(EvalValue::Boolean(
                    self.store
                        .relationships()
                        .has_relation(object, relation, subject),
                ))
            }
            // rebac::reachable(subject, relation, object, via, max_depth) -> bool
            // subject holds relation directly OR through groups reached via
            // its own `via` edges (bounded, cycle-safe).
            (Some("rebac"), "reachable") => {
                let Some((subject, relation, object)) =
                    self.rebac_ids_3(args, context, "reachable")?
                else {
                    // Unbound actor argument: the check is false, never an error.
                    return Ok(EvalValue::Boolean(false));
                };
                let (via, max) = self.rebac_via_max(args, context, "reachable")?;
                Ok(EvalValue::Boolean(
                    self.store
                        .relationships()
                        .has_relation_reachable(object, relation, subject, via, max),
                ))
            }
            // rebac::inherited(subject, relation, object, up, max_depth) -> bool
            // relation holds on the object or any ancestor along `up` edges.
            (Some("rebac"), "inherited") => {
                let Some((subject, relation, object)) =
                    self.rebac_ids_3(args, context, "inherited")?
                else {
                    // Unbound actor argument: the check is false, never an error.
                    return Ok(EvalValue::Boolean(false));
                };
                let (up, max) = self.rebac_via_max(args, context, "inherited")?;
                Ok(EvalValue::Boolean(
                    self.store
                        .relationships()
                        .has_relation_inherited(object, relation, subject, up, max),
                ))
            }

            // ===== Taint / context provenance (F1 agentic authz) =====
            // taint::level("key") -> "platform" | "verified" | "llm".
            // Reads the request's per-key provenance under the fail-untrusted
            // rule: taint mode off ⇒ everything is "platform"; taint mode on
            // ⇒ an unlabeled key is "llm" (an LLM-asserted attribute can never
            // masquerade as platform-derived).
            (Some("taint"), "level") => {
                let key = self.taint_key_arg(args, context)?;
                let level = match context.context_trust(&key) {
                    crate::TrustLevel::Platform => "platform",
                    crate::TrustLevel::Verified => "verified",
                    crate::TrustLevel::Llm => "llm",
                };
                Ok(EvalValue::String(level.to_string()))
            }
            // taint::trusted("key") -> bool: true iff the key is NOT
            // LLM-tainted (level >= verified). The common gate: reject any
            // attribute a possibly-injected model could have asserted.
            (Some("taint"), "trusted") => {
                let key = self.taint_key_arg(args, context)?;
                Ok(EvalValue::Boolean(
                    context.context_trust(&key) >= crate::TrustLevel::Verified,
                ))
            }

            (Some("regex"), "matches") => {
                if args.len() != 2 {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "regex::matches() requires exactly two arguments (text, pattern)"
                            .to_string(),
                    });
                }
                let text = match self.evaluate_expr(&args[0], context)? {
                    EvalValue::String(s) => s,
                    _ => {
                        return Err(ReaperError::InvalidPolicy {
                            reason: "regex::matches() first argument must be a string".to_string(),
                        })
                    }
                };
                let pattern = match self.evaluate_expr(&args[1], context)? {
                    EvalValue::String(s) => s,
                    _ => {
                        return Err(ReaperError::InvalidPolicy {
                            reason: "regex::matches() second argument must be a string (pattern)"
                                .to_string(),
                        })
                    }
                };
                let re = self.get_cached_regex(&pattern)?;
                Ok(EvalValue::Boolean(re.is_match(&text)))
            }

            (Some("regex"), "replace") => {
                if args.len() != 3 {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "regex::replace() requires exactly three arguments (text, pattern, replacement)"
                            .to_string(),
                    });
                }
                let text = match self.evaluate_expr(&args[0], context)? {
                    EvalValue::String(s) => s,
                    _ => {
                        return Err(ReaperError::InvalidPolicy {
                            reason: "regex::replace() first argument must be a string".to_string(),
                        })
                    }
                };
                let pattern = match self.evaluate_expr(&args[1], context)? {
                    EvalValue::String(s) => s,
                    _ => {
                        return Err(ReaperError::InvalidPolicy {
                            reason: "regex::replace() second argument must be a string (pattern)"
                                .to_string(),
                        })
                    }
                };
                let replacement = match self.evaluate_expr(&args[2], context)? {
                    EvalValue::String(s) => s,
                    _ => {
                        return Err(ReaperError::InvalidPolicy {
                            reason:
                                "regex::replace() third argument must be a string (replacement)"
                                    .to_string(),
                        })
                    }
                };
                let re = self.get_cached_regex(&pattern)?;
                Ok(EvalValue::String(
                    re.replace_all(&text, replacement.as_str()).to_string(),
                ))
            }

            (Some("regex"), "split") => {
                if args.len() != 2 {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "regex::split() requires exactly two arguments (text, pattern)"
                            .to_string(),
                    });
                }
                let text = match self.evaluate_expr(&args[0], context)? {
                    EvalValue::String(s) => s,
                    _ => {
                        return Err(ReaperError::InvalidPolicy {
                            reason: "regex::split() first argument must be a string".to_string(),
                        })
                    }
                };
                let pattern = match self.evaluate_expr(&args[1], context)? {
                    EvalValue::String(s) => s,
                    _ => {
                        return Err(ReaperError::InvalidPolicy {
                            reason: "regex::split() second argument must be a string (pattern)"
                                .to_string(),
                        })
                    }
                };
                let re = self.get_cached_regex(&pattern)?;
                let parts: Vec<EvalValue> = re
                    .split(&text)
                    .map(|s| EvalValue::String(s.to_string()))
                    .collect();
                Ok(EvalValue::Array(parts))
            }

            // Math namespace functions (using builtin_functions::math)
            (Some("math"), "abs") => {
                if args.len() != 1 {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "math::abs() requires exactly one argument".to_string(),
                    });
                }
                let value = self.evaluate_expr(&args[0], context)?;
                builtin_functions::math_abs(&value)
            }

            (Some("math"), "round") => {
                if args.len() != 1 {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "math::round() requires exactly one argument".to_string(),
                    });
                }
                let value = self.evaluate_expr(&args[0], context)?;
                builtin_functions::math_round(&value)
            }

            (Some("math"), "floor") => {
                if args.len() != 1 {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "math::floor() requires exactly one argument".to_string(),
                    });
                }
                let value = self.evaluate_expr(&args[0], context)?;
                builtin_functions::math_floor(&value)
            }

            (Some("math"), "ceil") => {
                if args.len() != 1 {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "math::ceil() requires exactly one argument".to_string(),
                    });
                }
                let value = self.evaluate_expr(&args[0], context)?;
                builtin_functions::math_ceil(&value)
            }

            (Some("math"), "sqrt") => {
                if args.len() != 1 {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "math::sqrt() requires exactly one argument".to_string(),
                    });
                }
                let value = self.evaluate_expr(&args[0], context)?;
                builtin_functions::math_sqrt(&value)
            }

            (Some("math"), "pow") => {
                if args.len() != 2 {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "math::pow() requires exactly two arguments (base, exponent)"
                            .to_string(),
                    });
                }
                let base = self.evaluate_expr(&args[0], context)?;
                let exp = self.evaluate_expr(&args[1], context)?;
                builtin_functions::math_pow(&base, &exp)
            }

            (Some("math"), "min") => {
                if args.len() != 2 {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "math::min() requires exactly two arguments".to_string(),
                    });
                }
                let a = self.evaluate_expr(&args[0], context)?;
                let b = self.evaluate_expr(&args[1], context)?;
                builtin_functions::math_min(&a, &b)
            }

            (Some("math"), "max") => {
                if args.len() != 2 {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "math::max() requires exactly two arguments".to_string(),
                    });
                }
                let a = self.evaluate_expr(&args[0], context)?;
                let b = self.evaluate_expr(&args[1], context)?;
                builtin_functions::math_max(&a, &b)
            }

            (Some("math"), "clamp") => {
                if args.len() != 3 {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "math::clamp() requires exactly three arguments (value, min, max)"
                            .to_string(),
                    });
                }
                let value = self.evaluate_expr(&args[0], context)?;
                let min_val = self.evaluate_expr(&args[1], context)?;
                let max_val = self.evaluate_expr(&args[2], context)?;
                builtin_functions::math_clamp(&value, &min_val, &max_val)
            }

            // JSON functions (using builtin_functions::json)
            (Some("json"), "parse") => {
                if args.is_empty() {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "json::parse() requires a JSON string argument".to_string(),
                    });
                }
                let value = self.evaluate_expr(&args[0], context)?;
                builtin_functions::json_parse(&value)
            }

            (Some("json"), "stringify") => {
                if args.is_empty() {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "json::stringify() requires a value argument".to_string(),
                    });
                }
                let value = self.evaluate_expr(&args[0], context)?;
                builtin_functions::json_stringify(&value)
            }

            (Some("json"), "is_valid") => {
                if args.is_empty() {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "json::is_valid() requires a string argument".to_string(),
                    });
                }
                let value = self.evaluate_expr(&args[0], context)?;
                builtin_functions::json_is_valid(&value)
            }

            _ => Err(ReaperError::InvalidPolicy {
                reason: format!(
                    "Unknown function: {}",
                    namespace
                        .map(|ns| format!("{}::{}", ns, function))
                        .unwrap_or_else(|| function.to_string())
                ),
            }),
        }
    }
}

impl super::ReapAstEvaluator {
    /// Evaluate the common (subject, relation, object) prefix of a rebac::*
    /// call into interned ids. Args are expressions, so any string-producing
    /// value works: the `user`/`resource` pseudo-variables, bound variables,
    /// entity attributes, or literals.
    /// Returns `Ok(None)` when an `actor` argument is unbound (the request
    /// carries no actor): the relationship check is then FALSE (fail closed),
    /// not an evaluation error — same semantics as the compiled evaluator.
    fn rebac_ids_3(
        &self,
        args: &[crate::reap::ast::Expr],
        context: &super::types::EvalContext,
        name: &str,
    ) -> Result<
        Option<(
            crate::data::EntityId,
            crate::data::interning::InternedString,
            crate::data::EntityId,
        )>,
        ReaperError,
    > {
        if args.len() < 3 {
            return Err(ReaperError::InvalidPolicy {
                reason: format!(
                    "rebac::{name} requires (subject, relation, object, ...), got {} args",
                    args.len()
                ),
            });
        }
        // `actor` is bound as a pseudo-variable only when the request carries
        // an actor (see rebac_pseudo_vars). Detect the unbound case up front.
        let unbound_actor = |e: &crate::reap::ast::Expr| {
            matches!(e, crate::reap::ast::Expr::Variable(n)
                if n == "actor" && !context.variables.contains_key("actor"))
        };
        if unbound_actor(&args[0]) || unbound_actor(&args[2]) {
            return Ok(None);
        }
        let subject = self.rebac_string_arg(&args[0], context, name, "subject")?;
        let relation = self.rebac_string_arg(&args[1], context, name, "relation")?;
        let object = self.rebac_string_arg(&args[2], context, name, "object")?;
        let interner = self.store.interner();
        Ok(Some((
            interner.intern(&subject),
            interner.intern(&relation),
            interner.intern(&object),
        )))
    }

    /// Evaluate the (via/up, max_depth) tail of traversing rebac::* calls.
    fn rebac_via_max(
        &self,
        args: &[crate::reap::ast::Expr],
        context: &super::types::EvalContext,
        name: &str,
    ) -> Result<(crate::data::interning::InternedString, usize), ReaperError> {
        if args.len() != 5 {
            return Err(ReaperError::InvalidPolicy {
                reason: format!(
                    "rebac::{name} requires (subject, relation, object, edge, max_depth), got {} args",
                    args.len()
                ),
            });
        }
        let via = self.rebac_string_arg(&args[3], context, name, "edge")?;
        let max = match self.evaluate_expr(&args[4], context)? {
            super::types::EvalValue::Integer(n) if n > 0 => n as usize,
            other => {
                return Err(ReaperError::InvalidPolicy {
                    reason: format!(
                        "rebac::{name} max_depth must be a positive integer, got {other:?}"
                    ),
                })
            }
        };
        // Explicit bound, clamped: traversal cost is capped by construction.
        Ok((self.store.interner().intern(&via), max.min(16)))
    }

    /// Resolve the single string key argument of a `taint::*` call.
    fn taint_key_arg(
        &self,
        args: &[crate::reap::ast::Expr],
        context: &super::types::EvalContext,
    ) -> Result<String, ReaperError> {
        if args.len() != 1 {
            return Err(ReaperError::InvalidPolicy {
                reason: format!("taint:: requires exactly (key), got {} args", args.len()),
            });
        }
        match self.evaluate_expr(&args[0], context)? {
            super::types::EvalValue::String(s) => Ok(s),
            other => Err(ReaperError::InvalidPolicy {
                reason: format!("taint:: key must be a string, got {other:?}"),
            }),
        }
    }

    fn rebac_string_arg(
        &self,
        arg: &crate::reap::ast::Expr,
        context: &super::types::EvalContext,
        name: &str,
        position: &str,
    ) -> Result<String, ReaperError> {
        match self.evaluate_expr(arg, context)? {
            super::types::EvalValue::String(s) => Ok(s),
            other => Err(ReaperError::InvalidPolicy {
                reason: format!("rebac::{name} {position} must be a string, got {other:?}"),
            }),
        }
    }
}
