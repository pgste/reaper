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
