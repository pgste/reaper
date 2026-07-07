//! Comprehension evaluation for AST evaluator.
//!
//! This module handles evaluation of set, array, and object comprehensions:
//! - Set comprehension: `{expr | var := collection; filters}`
//! - Array comprehension: `[expr | var := collection; filters]`
//! - Object comprehension: `{key: value | var := collection; filters}`

use super::types::{EvalContext, EvalValue};
use super::ReapAstEvaluator;
use crate::reap::ast::{
    AssignmentValue, Comprehension, ComprehensionIterator, Condition, Expr, IterationSource,
};
use reaper_core::ReaperError;
use std::collections::{HashMap, HashSet};

impl ReapAstEvaluator {
    /// Evaluate an assignment value (entity attribute, literal, variable, comprehension, or expression)
    pub(super) fn evaluate_assignment_value(
        &self,
        value: &AssignmentValue,
        context: &EvalContext,
    ) -> Result<EvalValue, ReaperError> {
        match value {
            AssignmentValue::EntityAttr(attr) => self.get_entity_attribute(attr, context),
            AssignmentValue::Value(val) => Ok(self.value_to_eval_value(val)),
            AssignmentValue::Variable(var_name) => context
                .variables
                .get(var_name)
                .cloned()
                .ok_or_else(|| ReaperError::InvalidPolicy {
                    reason: format!("Undefined variable: {}", var_name),
                }),
            AssignmentValue::Comprehension(comp) => self.evaluate_comprehension(comp, context),
            AssignmentValue::Expr(expr) => self.evaluate_expr(expr, context),
            AssignmentValue::Comparison { left, op, right } => {
                let result = self.evaluate_comparison(left, *op, right, context)?;
                Ok(EvalValue::Boolean(result))
            }
        }
    }

    /// Evaluate a comprehension (set, array, or object)
    pub(super) fn evaluate_comprehension(
        &self,
        comp: &Comprehension,
        context: &EvalContext,
    ) -> Result<EvalValue, ReaperError> {
        match comp {
            Comprehension::Set {
                output,
                iterator,
                filters,
            } => self.evaluate_set_comprehension(output, iterator, filters, context),

            Comprehension::Array {
                output,
                iterator,
                filters,
            } => self.evaluate_array_comprehension(output, iterator, filters, context),

            Comprehension::Object {
                key,
                value,
                iterator,
                filters,
            } => self.evaluate_object_comprehension(key, value, iterator, filters, context),
        }
    }

    /// Evaluate a set comprehension: `{expr | var := collection; filters}`
    fn evaluate_set_comprehension(
        &self,
        output: &Expr,
        iterator: &ComprehensionIterator,
        filters: &[Condition],
        context: &EvalContext,
    ) -> Result<EvalValue, ReaperError> {
        let items = self.get_iterator_items(iterator, context)?;
        let mut result_set = HashSet::with_capacity(items.len());

        for item in items {
            let mut item_context = context.clone();
            item_context
                .variables
                .insert(iterator.variable.clone(), item);

            // Check all filters
            let mut matches = true;
            for filter in filters {
                if !self.evaluate_condition(filter, &mut item_context)? {
                    matches = false;
                    break;
                }
            }

            if matches {
                let output_value = self.evaluate_expr(output, &item_context)?;
                result_set.insert(output_value);
            }
        }

        Ok(EvalValue::Set(result_set.into_iter().collect()))
    }

    /// Evaluate an array comprehension: `[expr | var := collection; filters]`
    fn evaluate_array_comprehension(
        &self,
        output: &Expr,
        iterator: &ComprehensionIterator,
        filters: &[Condition],
        context: &EvalContext,
    ) -> Result<EvalValue, ReaperError> {
        let items = self.get_iterator_items(iterator, context)?;
        let mut result = Vec::with_capacity(items.len());

        for item in items {
            let mut item_context = context.clone();
            item_context
                .variables
                .insert(iterator.variable.clone(), item);

            // Check all filters
            let mut matches = true;
            for filter in filters {
                if !self.evaluate_condition(filter, &mut item_context)? {
                    matches = false;
                    break;
                }
            }

            if matches {
                result.push(self.evaluate_expr(output, &item_context)?);
            }
        }

        Ok(EvalValue::Array(result))
    }

    /// Evaluate an object comprehension: `{key: value | var := collection; filters}`
    fn evaluate_object_comprehension(
        &self,
        key_expr: &Expr,
        value_expr: &Expr,
        iterator: &ComprehensionIterator,
        filters: &[Condition],
        context: &EvalContext,
    ) -> Result<EvalValue, ReaperError> {
        let items = self.get_iterator_items(iterator, context)?;
        let mut result = HashMap::with_capacity(items.len());

        for item in items {
            let mut item_context = context.clone();
            item_context
                .variables
                .insert(iterator.variable.clone(), item);

            // Check all filters
            let mut matches = true;
            for filter in filters {
                if !self.evaluate_condition(filter, &mut item_context)? {
                    matches = false;
                    break;
                }
            }

            if matches {
                let key = self.evaluate_expr(key_expr, &item_context)?;
                let value = self.evaluate_expr(value_expr, &item_context)?;

                if let EvalValue::String(key_str) = key {
                    result.insert(key_str, value);
                } else {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "Object comprehension key must be a string".to_string(),
                    });
                }
            }
        }

        Ok(EvalValue::Object(result))
    }

    /// Get items from an iterator collection
    pub(super) fn get_iterator_items(
        &self,
        iterator: &ComprehensionIterator,
        context: &EvalContext,
    ) -> Result<Vec<EvalValue>, ReaperError> {
        let collection =
            match &iterator.collection {
                IterationSource::EntityAttr(entity_attr) => {
                    self.get_entity_attribute(entity_attr, context)?
                }
                IterationSource::VarAttr(var_attr) => self.get_var_attribute(var_attr, context)?,
                IterationSource::IndexedVariable { variable, index } => {
                    let var_value = context.variables.get(variable).ok_or_else(|| {
                        ReaperError::InvalidPolicy {
                            reason: format!("Undefined variable in iteration: {}", variable),
                        }
                    })?;
                    self.apply_index(var_value, index)?
                }
            };

        match collection {
            EvalValue::Array(arr) | EvalValue::Set(arr) => Ok(arr),
            // Total iteration: a missing document path (Null) is an empty
            // collection, so document policies over absent/partial input fail
            // their rules instead of erroring the whole evaluation.
            EvalValue::Null => Ok(Vec::new()),
            _ => Err(ReaperError::InvalidPolicy {
                reason: "Iterator collection must be an array or set".to_string(),
            }),
        }
    }
}
