//! Method call dispatch for AST evaluator.
//!
//! This module handles dispatching method calls on values:
//! - Aggregate methods: count, sum, max, min, any, all
//! - String methods: lower, upper, trim, split, contains, startswith, endswith
//! - Regex methods: matches, find, find_all, replace
//! - Set operations: union, intersection, difference
//! - Collection methods: first, last, slice, reverse, sort, unique
//! - Object methods: keys, values, has_key

use super::builtin_methods;
use super::types::{EvalContext, EvalValue};
use super::ReapAstEvaluator;
use crate::reap::ast::{Expr, MethodName};
use reaper_core::ReaperError;

impl ReapAstEvaluator {
    /// Evaluate method call expressions (e.g., collection.count(), roles.sum())
    pub(super) fn evaluate_method_call(
        &self,
        receiver: &Expr,
        method: &MethodName,
        args: &[Expr],
        context: &EvalContext,
    ) -> Result<EvalValue, ReaperError> {
        let receiver_value = self.evaluate_expr(receiver, context)?;

        match method {
            // Aggregate methods (using builtin_methods)
            MethodName::Count => builtin_methods::method_count(&receiver_value),
            MethodName::Sum => {
                let items = builtin_methods::get_collection_items(&receiver_value)?;
                let owned: Vec<_> = items.into_iter().cloned().collect();
                builtin_methods::method_sum(&owned)
            }
            MethodName::Max => {
                let items = builtin_methods::get_collection_items(&receiver_value)?;
                let owned: Vec<_> = items.into_iter().cloned().collect();
                builtin_methods::method_max(&owned)
            }
            MethodName::Min => {
                let items = builtin_methods::get_collection_items(&receiver_value)?;
                let owned: Vec<_> = items.into_iter().cloned().collect();
                builtin_methods::method_min(&owned)
            }
            MethodName::Any => {
                let items = builtin_methods::get_collection_items(&receiver_value)?;
                let owned: Vec<_> = items.into_iter().cloned().collect();
                builtin_methods::method_any(&owned)
            }
            MethodName::All => {
                let items = builtin_methods::get_collection_items(&receiver_value)?;
                let owned: Vec<_> = items.into_iter().cloned().collect();
                builtin_methods::method_all(&owned)
            }

            // String methods (using builtin_methods)
            MethodName::Lower => builtin_methods::method_lower(&receiver_value),
            MethodName::Upper => builtin_methods::method_upper(&receiver_value),
            MethodName::Trim => builtin_methods::method_trim(&receiver_value),
            MethodName::Split => {
                if args.is_empty() {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "split() requires delimiter argument".to_string(),
                    });
                }
                let delimiter = self.evaluate_expr(&args[0], context)?;
                builtin_methods::method_split(&receiver_value, &delimiter)
            }
            MethodName::Contains => {
                if args.is_empty() {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "contains() requires substring argument".to_string(),
                    });
                }
                let substring = self.evaluate_expr(&args[0], context)?;
                builtin_methods::method_contains(&receiver_value, &substring)
            }
            MethodName::Startswith => {
                if args.is_empty() {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "startswith() requires prefix argument".to_string(),
                    });
                }
                let prefix = self.evaluate_expr(&args[0], context)?;
                builtin_methods::method_startswith(&receiver_value, &prefix)
            }
            MethodName::Endswith => {
                if args.is_empty() {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "endswith() requires suffix argument".to_string(),
                    });
                }
                let suffix = self.evaluate_expr(&args[0], context)?;
                builtin_methods::method_endswith(&receiver_value, &suffix)
            }

            // Regex methods (keep in evaluator due to cache dependency)
            MethodName::Matches => {
                if args.is_empty() {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "matches() requires pattern argument".to_string(),
                    });
                }
                let pattern = self.evaluate_expr(&args[0], context)?;
                self.method_matches(&receiver_value, &pattern)
            }
            MethodName::Find => {
                if args.is_empty() {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "find() requires pattern argument".to_string(),
                    });
                }
                let pattern = self.evaluate_expr(&args[0], context)?;
                self.method_find(&receiver_value, &pattern)
            }
            MethodName::FindAll => {
                if args.is_empty() {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "find_all() requires pattern argument".to_string(),
                    });
                }
                let pattern = self.evaluate_expr(&args[0], context)?;
                self.method_find_all(&receiver_value, &pattern)
            }
            MethodName::Replace => {
                if args.len() < 2 {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "replace() requires pattern and replacement arguments".to_string(),
                    });
                }
                let pattern = self.evaluate_expr(&args[0], context)?;
                let replacement = self.evaluate_expr(&args[1], context)?;
                self.method_replace(&receiver_value, &pattern, &replacement)
            }

            // Collection methods (using builtin_methods)
            MethodName::Union => {
                if args.is_empty() {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "union() requires another set argument".to_string(),
                    });
                }
                let other = self.evaluate_expr(&args[0], context)?;
                builtin_methods::method_union(&receiver_value, &other)
            }
            MethodName::Intersection => {
                if args.is_empty() {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "intersection() requires another set argument".to_string(),
                    });
                }
                let other = self.evaluate_expr(&args[0], context)?;
                builtin_methods::method_intersection(&receiver_value, &other)
            }
            MethodName::Difference => {
                if args.is_empty() {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "difference() requires another set argument".to_string(),
                    });
                }
                let other = self.evaluate_expr(&args[0], context)?;
                builtin_methods::method_difference(&receiver_value, &other)
            }

            // Advanced collection methods (using builtin_methods)
            MethodName::First => builtin_methods::method_first(&receiver_value),
            MethodName::Last => builtin_methods::method_last(&receiver_value),
            MethodName::Slice => {
                if args.len() < 2 {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "slice() requires start and end arguments".to_string(),
                    });
                }
                let start = self.evaluate_expr(&args[0], context)?;
                let end = self.evaluate_expr(&args[1], context)?;
                builtin_methods::method_slice(&receiver_value, &start, &end)
            }
            MethodName::Reverse => builtin_methods::method_reverse(&receiver_value),
            MethodName::Sort => builtin_methods::method_sort(&receiver_value),
            MethodName::Unique => builtin_methods::method_unique(&receiver_value),

            // Object methods (using builtin_methods)
            MethodName::Keys => builtin_methods::method_keys(&receiver_value),
            MethodName::Values => builtin_methods::method_values(&receiver_value),
            MethodName::HasKey => {
                if args.is_empty() {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "has_key() requires a key argument".to_string(),
                    });
                }
                let key = self.evaluate_expr(&args[0], context)?;
                builtin_methods::method_has_key(&receiver_value, &key)
            }
        }
    }
}
