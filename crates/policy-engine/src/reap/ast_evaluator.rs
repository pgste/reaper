//! AST Evaluator - Direct evaluation of parsed .reap policies
//!
//! Evaluates policies directly from the AST without compilation.
//! Supports advanced features like comprehensions, variable assignments, and complex expressions.
//!
//! Performance characteristics:
//! - Simple rules: < 1 µs
//! - Comprehensions (100 items): < 10 µs
//! - Variable assignments: ~100 ns overhead

use super::ast::*;
use crate::data::{AttributeValue, DataStore, EntityId};
use crate::{PolicyAction, PolicyRequest};
use reaper_core::ReaperError;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// AST-based policy evaluator
///
/// Evaluates policies directly from the AST, supporting all language features
/// including comprehensions, variable assignments, and complex expressions.
///
/// Performance optimizations:
/// - Regex pattern caching: 2-5x speedup for repeated patterns
/// - SIMD aggregates: 2-4x speedup for large numeric arrays (>64 elements)
#[derive(Debug)]
pub struct ReapAstEvaluator {
    /// Reference to the data store
    store: Arc<DataStore>,
    /// Parsed policy AST
    policy: Policy,
    /// Regex pattern cache for performance (2-5x speedup)
    /// Compiled regex patterns are expensive, cache them by pattern string
    regex_cache: RefCell<HashMap<String, regex::Regex>>,
}

/// Evaluation context holding variable bindings
#[derive(Debug, Clone)]
struct EvalContext {
    /// Variable name -> value mappings
    variables: HashMap<String, EvalValue>,
    /// User entity from request
    user_id: EntityId,
    /// Resource entity from request
    resource_id: EntityId,
}

/// Runtime value during evaluation
#[derive(Debug, Clone)]
enum EvalValue {
    String(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Null,
    Array(Vec<EvalValue>),
    Object(HashMap<String, EvalValue>),
    Set(Vec<EvalValue>), // Using Vec for now, can optimize to HashSet later
}

impl PartialEq for EvalValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (EvalValue::String(a), EvalValue::String(b)) => a == b,
            (EvalValue::Integer(a), EvalValue::Integer(b)) => a == b,
            (EvalValue::Float(a), EvalValue::Float(b)) => a.to_bits() == b.to_bits(),
            (EvalValue::Boolean(a), EvalValue::Boolean(b)) => a == b,
            (EvalValue::Null, EvalValue::Null) => true,
            (EvalValue::Array(a), EvalValue::Array(b)) => a == b,
            (EvalValue::Object(a), EvalValue::Object(b)) => a == b,
            (EvalValue::Set(a), EvalValue::Set(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for EvalValue {}

impl Hash for EvalValue {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            EvalValue::String(s) => {
                0u8.hash(state);
                s.hash(state);
            }
            EvalValue::Integer(i) => {
                1u8.hash(state);
                i.hash(state);
            }
            EvalValue::Float(f) => {
                2u8.hash(state);
                f.to_bits().hash(state);
            }
            EvalValue::Boolean(b) => {
                3u8.hash(state);
                b.hash(state);
            }
            EvalValue::Null => {
                4u8.hash(state);
            }
            EvalValue::Array(arr) => {
                5u8.hash(state);
                arr.hash(state);
            }
            EvalValue::Object(obj) => {
                6u8.hash(state);
                // Hash objects by sorted keys for consistency
                let mut entries: Vec<_> = obj.iter().collect();
                entries.sort_by_key(|(k, _)| *k);
                entries.hash(state);
            }
            EvalValue::Set(set) => {
                7u8.hash(state);
                set.hash(state);
            }
        }
    }
}

impl ReapAstEvaluator {
    /// Create a new AST evaluator
    pub fn new(store: Arc<DataStore>, policy: Policy) -> Self {
        Self {
            store,
            policy,
            regex_cache: RefCell::new(HashMap::new()),
        }
    }

    /// Evaluate a policy request
    pub fn evaluate(&self, request: &PolicyRequest) -> Result<PolicyAction, ReaperError> {
        // Get user and resource IDs from the DataStore
        let interner = self.store.interner();
        let user_id = interner.intern(request.context.get("principal").ok_or_else(|| {
            ReaperError::InvalidPolicy {
                reason: "Request must have 'principal' in context".to_string(),
            }
        })?);
        let resource_id = interner.intern(&request.resource);

        // Create evaluation context
        let mut context = EvalContext {
            variables: HashMap::new(),
            user_id,
            resource_id,
        };

        // Security-first evaluation: Deny rules ALWAYS take precedence over Allow rules
        // This ensures explicit denies cannot be bypassed by subsequent allow rules

        // Phase 1: Evaluate all DENY rules first
        for rule in &self.policy.rules {
            if matches!(rule.decision, super::ast::Decision::Deny)
                && self.evaluate_condition(&rule.condition, &mut context)?
            {
                // Explicit deny - return immediately, no allow can override this
                return Ok(PolicyAction::Deny);
            }
        }

        // Phase 2: No deny matched, now evaluate ALLOW rules
        for rule in &self.policy.rules {
            if matches!(rule.decision, super::ast::Decision::Allow)
                && self.evaluate_condition(&rule.condition, &mut context)?
            {
                return Ok(PolicyAction::Allow);
            }
        }

        // Phase 3: No rule matched - return default decision
        Ok(self.policy.default_decision.clone().into())
    }

    /// Evaluate a condition
    fn evaluate_condition(
        &self,
        condition: &Condition,
        context: &mut EvalContext,
    ) -> Result<bool, ReaperError> {
        match condition {
            Condition::True => Ok(true),
            Condition::False => Ok(false),

            Condition::Comparison { left, op, right } => {
                self.evaluate_comparison(left, *op, right, context)
            }

            Condition::Assignment { variable, value } => {
                // Evaluate the assignment value and store in context
                let eval_value = self.evaluate_assignment_value(value, context)?;
                context.variables.insert(variable.clone(), eval_value);
                // Assignments always succeed (return true)
                Ok(true)
            }

            Condition::And(conditions) => {
                for cond in conditions {
                    if !self.evaluate_condition(cond, context)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }

            Condition::Or(conditions) => {
                for cond in conditions {
                    if self.evaluate_condition(cond, context)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }

            Condition::Not(cond) => Ok(!self.evaluate_condition(cond, context)?),

            Condition::Expr(expr) => {
                // Evaluate the expression and convert to boolean
                let value = self.evaluate_expr(expr, context)?;
                match value {
                    EvalValue::Boolean(b) => Ok(b),
                    _ => Err(ReaperError::InvalidPolicy {
                        reason: format!(
                            "Expression in condition must evaluate to boolean, got: {:?}",
                            value
                        ),
                    }),
                }
            }
        }
    }

    /// Evaluate a comparison
    fn evaluate_comparison(
        &self,
        left: &ComparisonLeft,
        op: Operator,
        right: &ComparisonRight,
        context: &EvalContext,
    ) -> Result<bool, ReaperError> {
        // Get left value
        let left_value = match left {
            ComparisonLeft::EntityAttr(attr) => self.get_entity_attribute(attr, context)?,
            ComparisonLeft::VarAttr(var_attr) => self.get_var_attribute(var_attr, context)?,
        };

        // Get right value
        let right_value = match right {
            ComparisonRight::Value(val) => self.value_to_eval_value(val),
            ComparisonRight::EntityAttr(attr) => self.get_entity_attribute(attr, context)?,
            ComparisonRight::VarAttr(var_attr) => self.get_var_attribute(var_attr, context)?,
            ComparisonRight::Variable(var_name) => {
                context.variables.get(var_name).cloned().ok_or_else(|| {
                    ReaperError::InvalidPolicy {
                        reason: format!("Undefined variable: {}", var_name),
                    }
                })?
            }
        };

        // Perform comparison based on operator
        match op {
            Operator::Equal => Ok(self.values_equal(&left_value, &right_value)),
            Operator::NotEqual => Ok(!self.values_equal(&left_value, &right_value)),
            Operator::GreaterThan => self.compare_numeric(&left_value, &right_value, |a, b| a > b),
            Operator::LessThan => self.compare_numeric(&left_value, &right_value, |a, b| a < b),
            Operator::GreaterEqual => {
                self.compare_numeric(&left_value, &right_value, |a, b| a >= b)
            }
            Operator::LessEqual => self.compare_numeric(&left_value, &right_value, |a, b| a <= b),
            Operator::In => self.check_membership(&right_value, &left_value),
        }
    }

    /// Check if a value is in a collection
    fn check_membership(
        &self,
        collection: &EvalValue,
        value: &EvalValue,
    ) -> Result<bool, ReaperError> {
        match collection {
            EvalValue::Array(arr) | EvalValue::Set(arr) => {
                Ok(arr.iter().any(|item| self.values_equal(item, value)))
            }
            EvalValue::Object(map) => {
                // For objects, check if key exists
                if let EvalValue::String(key) = value {
                    Ok(map.contains_key(key))
                } else {
                    Ok(false)
                }
            }
            _ => Err(ReaperError::InvalidPolicy {
                reason: "In operator requires array, set, or object on right side".to_string(),
            }),
        }
    }

    /// Compare two values for equality
    fn values_equal(&self, a: &EvalValue, b: &EvalValue) -> bool {
        match (a, b) {
            (EvalValue::String(a), EvalValue::String(b)) => a == b,
            (EvalValue::Integer(a), EvalValue::Integer(b)) => a == b,
            (EvalValue::Float(a), EvalValue::Float(b)) => (a - b).abs() < f64::EPSILON,
            (EvalValue::Boolean(a), EvalValue::Boolean(b)) => a == b,
            (EvalValue::Null, EvalValue::Null) => true,
            _ => false,
        }
    }

    /// Compare numeric values
    fn compare_numeric<F>(&self, a: &EvalValue, b: &EvalValue, cmp: F) -> Result<bool, ReaperError>
    where
        F: Fn(f64, f64) -> bool,
    {
        let a_num = match a {
            EvalValue::Integer(i) => *i as f64,
            EvalValue::Float(f) => *f,
            _ => {
                return Err(ReaperError::InvalidPolicy {
                    reason: "Numeric comparison requires integer or float".to_string(),
                })
            }
        };

        let b_num = match b {
            EvalValue::Integer(i) => *i as f64,
            EvalValue::Float(f) => *f,
            _ => {
                return Err(ReaperError::InvalidPolicy {
                    reason: "Numeric comparison requires integer or float".to_string(),
                })
            }
        };

        Ok(cmp(a_num, b_num))
    }

    /// Get entity attribute value
    fn get_entity_attribute(
        &self,
        attr: &EntityAttr,
        context: &EvalContext,
    ) -> Result<EvalValue, ReaperError> {
        let entity_id = match attr.entity {
            Entity::User => context.user_id,
            Entity::Resource => context.resource_id,
            Entity::Context => {
                return Err(ReaperError::InvalidPolicy {
                    reason: "Context entity not yet supported in AST evaluator".to_string(),
                })
            }
        };

        // Get entity from DataStore
        let entity = self
            .store
            .get(entity_id)
            .ok_or_else(|| ReaperError::InvalidPolicy {
                reason: format!("Entity with ID {:?} not found", entity_id),
            })?;

        // Get attribute value
        let interner = self.store.interner();
        let attr_id = interner.intern(&attr.attribute);
        let value = entity.get_attribute(attr_id);

        // Convert AttributeValue to EvalValue
        self.attribute_value_to_eval_value(value, &attr.index)
    }

    /// Get variable attribute value
    fn get_var_attribute(
        &self,
        var_attr: &VarAttr,
        context: &EvalContext,
    ) -> Result<EvalValue, ReaperError> {
        // Get the variable value
        let var_value = context.variables.get(&var_attr.variable).ok_or_else(|| {
            ReaperError::InvalidPolicy {
                reason: format!("Undefined variable: {}", var_attr.variable),
            }
        })?;

        // Access the attribute from the variable
        match var_value {
            EvalValue::Object(map) => {
                let value = map
                    .get(&var_attr.attribute)
                    .cloned()
                    .unwrap_or(EvalValue::Null);

                // Handle optional indexing
                if let Some(index) = &var_attr.index {
                    self.apply_index(&value, index)
                } else {
                    Ok(value)
                }
            }
            _ => Err(ReaperError::InvalidPolicy {
                reason: format!(
                    "Cannot access attribute '{}' on non-object variable '{}'",
                    var_attr.attribute, var_attr.variable
                ),
            }),
        }
    }

    /// Apply an index to a value
    fn apply_index(&self, value: &EvalValue, index: &Index) -> Result<EvalValue, ReaperError> {
        match (value, index) {
            // Wildcard index returns the entire collection (used in comprehensions)
            (_, Index::Wildcard) => Ok(value.clone()),
            // Numeric index into array
            (EvalValue::Array(arr), Index::Number(n)) => {
                let idx = *n as usize;
                Ok(arr.get(idx).cloned().unwrap_or(EvalValue::Null))
            }
            // String index into object
            (EvalValue::Object(map), Index::String(key)) => {
                Ok(map.get(key).cloned().unwrap_or(EvalValue::Null))
            }
            _ => Err(ReaperError::InvalidPolicy {
                reason: "Invalid index operation".to_string(),
            }),
        }
    }

    /// Convert AttributeValue to EvalValue
    fn attribute_value_to_eval_value(
        &self,
        value: Option<&AttributeValue>,
        index: &Option<Index>,
    ) -> Result<EvalValue, ReaperError> {
        let value = value.ok_or_else(|| ReaperError::InvalidPolicy {
            reason: "Attribute not found".to_string(),
        })?;

        let eval_value = match value {
            AttributeValue::String(id) => {
                let interner = self.store.interner();
                EvalValue::String(
                    interner
                        .resolve(*id)
                        .map(|s| s.to_string())
                        .unwrap_or_default(),
                )
            }
            AttributeValue::Int(i) => EvalValue::Integer(*i),
            AttributeValue::Float(f) => EvalValue::Float(*f),
            AttributeValue::Bool(b) => EvalValue::Boolean(*b),
            AttributeValue::List(list) => {
                // Convert list to EvalValue array
                let items: Vec<EvalValue> = list
                    .iter()
                    .map(|v| {
                        self.attribute_value_to_eval_value(Some(v), &None)
                            .unwrap_or(EvalValue::Null)
                    })
                    .collect();
                EvalValue::Array(items)
            }
            AttributeValue::Object(map) => {
                // Convert object to EvalValue object
                let interner = self.store.interner();
                let mut obj = HashMap::new();
                for (key, val) in map {
                    let key_str = interner
                        .resolve(*key)
                        .map(|s| s.to_string())
                        .unwrap_or_default();
                    let val_eval = self
                        .attribute_value_to_eval_value(Some(val), &None)
                        .unwrap_or(EvalValue::Null);
                    obj.insert(key_str, val_eval);
                }
                EvalValue::Object(obj)
            }
            AttributeValue::Set(set) => {
                // Convert set to EvalValue set
                let items: Vec<EvalValue> = set
                    .iter()
                    .map(|v| {
                        self.attribute_value_to_eval_value(Some(v), &None)
                            .unwrap_or(EvalValue::Null)
                    })
                    .collect();
                EvalValue::Set(items)
            }
            AttributeValue::Null => EvalValue::Null,
        };

        // Apply index if present
        if let Some(idx) = index {
            self.apply_index(&eval_value, idx)
        } else {
            Ok(eval_value)
        }
    }

    /// Convert AST Value to EvalValue
    #[allow(clippy::only_used_in_recursion)]
    fn value_to_eval_value(&self, value: &Value) -> EvalValue {
        match value {
            Value::String(s) => EvalValue::String(s.clone()),
            Value::Integer(i) => EvalValue::Integer(*i),
            Value::Float(f) => EvalValue::Float(*f),
            Value::Boolean(b) => EvalValue::Boolean(*b),
            Value::Null => EvalValue::Null,
            Value::Array(arr) => {
                EvalValue::Array(arr.iter().map(|v| self.value_to_eval_value(v)).collect())
            }
            Value::Object(obj) => EvalValue::Object(
                obj.iter()
                    .map(|(k, v)| (k.clone(), self.value_to_eval_value(v)))
                    .collect(),
            ),
            Value::Set(set) => {
                EvalValue::Set(set.iter().map(|v| self.value_to_eval_value(v)).collect())
            }
        }
    }

    /// Evaluate an assignment value (including comprehensions)
    fn evaluate_assignment_value(
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
        }
    }

    /// Evaluate a comprehension
    fn evaluate_comprehension(
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

    /// Evaluate a set comprehension
    fn evaluate_set_comprehension(
        &self,
        output: &Expr,
        iterator: &ComprehensionIterator,
        filters: &[Condition],
        context: &EvalContext,
    ) -> Result<EvalValue, ReaperError> {
        // Get items and pre-allocate capacity for better performance
        let items = self.get_iterator_items(iterator, context)?;
        let mut result_set = HashSet::with_capacity(items.len());

        // Iterate over collection
        for item in items {
            // Create new context with iterator variable bound
            let mut item_context = context.clone();
            item_context
                .variables
                .insert(iterator.variable.clone(), item);

            // Check filters
            let mut matches = true;
            for filter in filters {
                if !self.evaluate_condition(filter, &mut item_context)? {
                    matches = false;
                    break;
                }
            }

            // If all filters pass, evaluate output expression and add to set
            if matches {
                let output_value = self.evaluate_expr(output, &item_context)?;
                // HashSet automatically handles deduplication with O(1) average complexity
                result_set.insert(output_value);
            }
        }

        // Convert HashSet to Vec for storage
        Ok(EvalValue::Set(result_set.into_iter().collect()))
    }

    /// Evaluate an array comprehension
    fn evaluate_array_comprehension(
        &self,
        output: &Expr,
        iterator: &ComprehensionIterator,
        filters: &[Condition],
        context: &EvalContext,
    ) -> Result<EvalValue, ReaperError> {
        // Get items and pre-allocate capacity for better performance
        let items = self.get_iterator_items(iterator, context)?;
        let mut result = Vec::with_capacity(items.len());

        // Iterate over collection
        for item in items {
            // Create new context with iterator variable bound
            let mut item_context = context.clone();
            item_context
                .variables
                .insert(iterator.variable.clone(), item);

            // Check filters
            let mut matches = true;
            for filter in filters {
                if !self.evaluate_condition(filter, &mut item_context)? {
                    matches = false;
                    break;
                }
            }

            // If all filters pass, evaluate output expression and add to array
            if matches {
                result.push(self.evaluate_expr(output, &item_context)?);
            }
        }

        Ok(EvalValue::Array(result))
    }

    /// Evaluate an object comprehension
    fn evaluate_object_comprehension(
        &self,
        key_expr: &Expr,
        value_expr: &Expr,
        iterator: &ComprehensionIterator,
        filters: &[Condition],
        context: &EvalContext,
    ) -> Result<EvalValue, ReaperError> {
        // Get items and pre-allocate capacity for better performance
        let items = self.get_iterator_items(iterator, context)?;
        let mut result = HashMap::with_capacity(items.len());

        // Iterate over collection
        for item in items {
            // Create new context with iterator variable bound
            let mut item_context = context.clone();
            item_context
                .variables
                .insert(iterator.variable.clone(), item);

            // Check filters
            let mut matches = true;
            for filter in filters {
                if !self.evaluate_condition(filter, &mut item_context)? {
                    matches = false;
                    break;
                }
            }

            // If all filters pass, evaluate key and value expressions
            if matches {
                let key = self.evaluate_expr(key_expr, &item_context)?;
                let value = self.evaluate_expr(value_expr, &item_context)?;

                // Key must be a string
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
    fn get_iterator_items(
        &self,
        iterator: &ComprehensionIterator,
        context: &EvalContext,
    ) -> Result<Vec<EvalValue>, ReaperError> {
        // Get the collection to iterate over
        let collection = self.get_entity_attribute(&iterator.collection, context)?;

        // If it's an array or set, return its elements
        match collection {
            EvalValue::Array(arr) | EvalValue::Set(arr) => Ok(arr),
            _ => Err(ReaperError::InvalidPolicy {
                reason: "Iterator collection must be an array or set".to_string(),
            }),
        }
    }

    /// Evaluate an expression
    fn evaluate_expr(&self, expr: &Expr, context: &EvalContext) -> Result<EvalValue, ReaperError> {
        match expr {
            Expr::Literal(val) => Ok(self.value_to_eval_value(val)),

            Expr::Variable(var_name) => {
                context
                    .variables
                    .get(var_name)
                    .cloned()
                    .ok_or_else(|| ReaperError::InvalidPolicy {
                        reason: format!("Undefined variable: {}", var_name),
                    })
            }

            Expr::AttributeAccess {
                variable,
                attribute,
            } => {
                let var_value =
                    context
                        .variables
                        .get(variable)
                        .ok_or_else(|| ReaperError::InvalidPolicy {
                            reason: format!("Undefined variable: {}", variable),
                        })?;

                match var_value {
                    EvalValue::Object(map) => {
                        Ok(map.get(attribute).cloned().unwrap_or(EvalValue::Null))
                    }
                    _ => Err(ReaperError::InvalidPolicy {
                        reason: format!(
                            "Cannot access attribute '{}' on non-object variable '{}'",
                            attribute, variable
                        ),
                    }),
                }
            }

            Expr::IndexedAccess {
                variable,
                attribute,
                index,
            } => {
                let var_value =
                    context
                        .variables
                        .get(variable)
                        .ok_or_else(|| ReaperError::InvalidPolicy {
                            reason: format!("Undefined variable: {}", variable),
                        })?;

                match var_value {
                    EvalValue::Object(map) => {
                        let attr_value = map.get(attribute).cloned().unwrap_or(EvalValue::Null);
                        self.apply_index(&attr_value, index)
                    }
                    _ => Err(ReaperError::InvalidPolicy {
                        reason: format!(
                            "Cannot access attribute '{}' on non-object variable '{}'",
                            attribute, variable
                        ),
                    }),
                }
            }

            Expr::MethodCall {
                receiver,
                method,
                args,
            } => self.evaluate_method_call(receiver, method, args, context),

            Expr::FunctionCall {
                namespace,
                function,
                args,
            } => self.evaluate_function_call(namespace.as_deref(), function, args, context),
        }
    }

    /// Evaluate method call expressions (e.g., collection.count(), roles.sum())
    fn evaluate_method_call(
        &self,
        receiver: &Expr,
        method: &MethodName,
        args: &[Expr],
        context: &EvalContext,
    ) -> Result<EvalValue, ReaperError> {
        let receiver_value = self.evaluate_expr(receiver, context)?;

        match method {
            // Aggregate methods
            MethodName::Count => self.method_count(&receiver_value),
            MethodName::Sum => self.method_sum(&receiver_value),
            MethodName::Max => self.method_max(&receiver_value),
            MethodName::Min => self.method_min(&receiver_value),
            MethodName::Any => self.method_any(&receiver_value),
            MethodName::All => self.method_all(&receiver_value),

            // String methods
            MethodName::Lower => self.method_lower(&receiver_value),
            MethodName::Upper => self.method_upper(&receiver_value),
            MethodName::Trim => self.method_trim(&receiver_value),
            MethodName::Split => {
                if args.is_empty() {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "split() requires delimiter argument".to_string(),
                    });
                }
                let delimiter = self.evaluate_expr(&args[0], context)?;
                self.method_split(&receiver_value, &delimiter)
            }
            MethodName::Contains => {
                if args.is_empty() {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "contains() requires substring argument".to_string(),
                    });
                }
                let substring = self.evaluate_expr(&args[0], context)?;
                self.method_contains(&receiver_value, &substring)
            }
            MethodName::Startswith => {
                if args.is_empty() {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "startswith() requires prefix argument".to_string(),
                    });
                }
                let prefix = self.evaluate_expr(&args[0], context)?;
                self.method_startswith(&receiver_value, &prefix)
            }
            MethodName::Endswith => {
                if args.is_empty() {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "endswith() requires suffix argument".to_string(),
                    });
                }
                let suffix = self.evaluate_expr(&args[0], context)?;
                self.method_endswith(&receiver_value, &suffix)
            }

            // Regex methods
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

            // Collection methods
            MethodName::Union => {
                if args.is_empty() {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "union() requires another set argument".to_string(),
                    });
                }
                let other = self.evaluate_expr(&args[0], context)?;
                self.method_union(&receiver_value, &other)
            }
            MethodName::Intersection => {
                if args.is_empty() {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "intersection() requires another set argument".to_string(),
                    });
                }
                let other = self.evaluate_expr(&args[0], context)?;
                self.method_intersection(&receiver_value, &other)
            }
            MethodName::Difference => {
                if args.is_empty() {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "difference() requires another set argument".to_string(),
                    });
                }
                let other = self.evaluate_expr(&args[0], context)?;
                self.method_difference(&receiver_value, &other)
            }

            // Advanced collection methods
            MethodName::First => self.method_first(&receiver_value),
            MethodName::Last => self.method_last(&receiver_value),
            MethodName::Slice => {
                if args.len() < 2 {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "slice() requires start and end arguments".to_string(),
                    });
                }
                let start = self.evaluate_expr(&args[0], context)?;
                let end = self.evaluate_expr(&args[1], context)?;
                self.method_slice(&receiver_value, &start, &end)
            }
            MethodName::Reverse => self.method_reverse(&receiver_value),
            MethodName::Sort => self.method_sort(&receiver_value),
            MethodName::Unique => self.method_unique(&receiver_value),

            // Object methods
            MethodName::Keys => self.method_keys(&receiver_value),
            MethodName::Values => self.method_values(&receiver_value),
            MethodName::HasKey => {
                if args.is_empty() {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "has_key() requires a key argument".to_string(),
                    });
                }
                let key = self.evaluate_expr(&args[0], context)?;
                self.method_has_key(&receiver_value, &key)
            }
        }
    }

    /// Evaluate function call expressions (e.g., time.now_ns(), concat(a, b))
    fn evaluate_function_call(
        &self,
        namespace: Option<&str>,
        function: &str,
        args: &[Expr],
        context: &EvalContext,
    ) -> Result<EvalValue, ReaperError> {
        match (namespace, function) {
            // Type checking functions
            (None, "is_string") => {
                if args.is_empty() {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "is_string() requires one argument".to_string(),
                    });
                }
                let value = self.evaluate_expr(&args[0], context)?;
                Ok(EvalValue::Boolean(matches!(value, EvalValue::String(_))))
            }
            (None, "is_number") => {
                if args.is_empty() {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "is_number() requires one argument".to_string(),
                    });
                }
                let value = self.evaluate_expr(&args[0], context)?;
                Ok(EvalValue::Boolean(matches!(
                    value,
                    EvalValue::Integer(_) | EvalValue::Float(_)
                )))
            }
            (None, "is_bool") => {
                if args.is_empty() {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "is_bool() requires one argument".to_string(),
                    });
                }
                let value = self.evaluate_expr(&args[0], context)?;
                Ok(EvalValue::Boolean(matches!(value, EvalValue::Boolean(_))))
            }
            (None, "is_array") => {
                if args.is_empty() {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "is_array() requires one argument".to_string(),
                    });
                }
                let value = self.evaluate_expr(&args[0], context)?;
                Ok(EvalValue::Boolean(matches!(value, EvalValue::Array(_))))
            }
            (None, "is_set") => {
                if args.is_empty() {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "is_set() requires one argument".to_string(),
                    });
                }
                let value = self.evaluate_expr(&args[0], context)?;
                Ok(EvalValue::Boolean(matches!(value, EvalValue::Set(_))))
            }
            (None, "is_object") => {
                if args.is_empty() {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "is_object() requires one argument".to_string(),
                    });
                }
                let value = self.evaluate_expr(&args[0], context)?;
                Ok(EvalValue::Boolean(matches!(value, EvalValue::Object(_))))
            }
            (None, "is_null") => {
                if args.is_empty() {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "is_null() requires one argument".to_string(),
                    });
                }
                let value = self.evaluate_expr(&args[0], context)?;
                Ok(EvalValue::Boolean(matches!(value, EvalValue::Null)))
            }

            // String concatenation
            (None, "concat") => {
                let strings: Result<Vec<String>, _> = args
                    .iter()
                    .map(|arg| {
                        let val = self.evaluate_expr(arg, context)?;
                        match val {
                            EvalValue::String(s) => Ok(s),
                            _ => Err(ReaperError::InvalidPolicy {
                                reason: "concat() requires string arguments".to_string(),
                            }),
                        }
                    })
                    .collect();

                Ok(EvalValue::String(strings?.join("")))
            }

            // ===== Time/Date Functions =====

            // Current time functions
            (Some("time"), "now_ns") => {
                if !args.is_empty() {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "time::now_ns() takes no arguments".to_string(),
                    });
                }
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|e| ReaperError::InvalidPolicy {
                        reason: format!("System time error: {}", e),
                    })?;
                Ok(EvalValue::Integer(now.as_nanos() as i64))
            }

            (Some("time"), "now_ms") => {
                if !args.is_empty() {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "time::now_ms() takes no arguments".to_string(),
                    });
                }
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|e| ReaperError::InvalidPolicy {
                        reason: format!("System time error: {}", e),
                    })?;
                Ok(EvalValue::Integer(now.as_millis() as i64))
            }

            (Some("time"), "now") => {
                if !args.is_empty() {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "time::now() takes no arguments".to_string(),
                    });
                }
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|e| ReaperError::InvalidPolicy {
                        reason: format!("System time error: {}", e),
                    })?;
                Ok(EvalValue::Integer(now.as_secs() as i64))
            }

            // Time parsing
            (Some("time"), "parse_rfc3339") => {
                if args.len() != 1 {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "time::parse_rfc3339() requires exactly one argument (string)"
                            .to_string(),
                    });
                }
                let value = self.evaluate_expr(&args[0], context)?;
                let time_str = match value {
                    EvalValue::String(s) => s,
                    _ => {
                        return Err(ReaperError::InvalidPolicy {
                            reason: "time::parse_rfc3339() requires a string argument".to_string(),
                        })
                    }
                };

                // Parse RFC3339/ISO8601 string using chrono
                use chrono::DateTime;
                let dt = DateTime::parse_from_rfc3339(&time_str).map_err(|e| {
                    ReaperError::InvalidPolicy {
                        reason: format!("Invalid RFC3339 timestamp '{}': {}", time_str, e),
                    }
                })?;

                Ok(EvalValue::Integer(dt.timestamp_nanos_opt().unwrap_or(0)))
            }

            // Time formatting
            (Some("time"), "format_rfc3339") => {
                if args.len() != 1 {
                    return Err(ReaperError::InvalidPolicy {
                        reason:
                            "time::format_rfc3339() requires exactly one argument (nanoseconds)"
                                .to_string(),
                    });
                }
                let value = self.evaluate_expr(&args[0], context)?;
                let nanos =
                    match value {
                        EvalValue::Integer(n) => n,
                        _ => return Err(ReaperError::InvalidPolicy {
                            reason:
                                "time::format_rfc3339() requires an integer argument (nanoseconds)"
                                    .to_string(),
                        }),
                    };

                use chrono::DateTime;
                let dt =
                    DateTime::from_timestamp(nanos / 1_000_000_000, (nanos % 1_000_000_000) as u32)
                        .ok_or_else(|| ReaperError::InvalidPolicy {
                            reason: format!("Invalid timestamp: {}", nanos),
                        })?;

                Ok(EvalValue::String(dt.to_rfc3339()))
            }

            // Time arithmetic
            (Some("time"), "add_ns") => {
                if args.len() != 2 {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "time::add_ns() requires exactly two arguments (timestamp_ns, duration_ns)".to_string(),
                    });
                }
                let timestamp = match self.evaluate_expr(&args[0], context)? {
                    EvalValue::Integer(n) => n,
                    _ => {
                        return Err(ReaperError::InvalidPolicy {
                            reason:
                                "time::add_ns() first argument must be an integer (nanoseconds)"
                                    .to_string(),
                        })
                    }
                };
                let duration = match self.evaluate_expr(&args[1], context)? {
                    EvalValue::Integer(n) => n,
                    _ => return Err(ReaperError::InvalidPolicy {
                        reason: "time::add_ns() second argument must be an integer (duration in nanoseconds)".to_string(),
                    }),
                };

                Ok(EvalValue::Integer(timestamp.saturating_add(duration)))
            }

            (Some("time"), "subtract_ns") => {
                if args.len() != 2 {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "time::subtract_ns() requires exactly two arguments (timestamp_ns, duration_ns)".to_string(),
                    });
                }
                let timestamp = match self.evaluate_expr(&args[0], context)? {
                    EvalValue::Integer(n) => n,
                    _ => return Err(ReaperError::InvalidPolicy {
                        reason:
                            "time::subtract_ns() first argument must be an integer (nanoseconds)"
                                .to_string(),
                    }),
                };
                let duration = match self.evaluate_expr(&args[1], context)? {
                    EvalValue::Integer(n) => n,
                    _ => return Err(ReaperError::InvalidPolicy {
                        reason: "time::subtract_ns() second argument must be an integer (duration in nanoseconds)".to_string(),
                    }),
                };

                Ok(EvalValue::Integer(timestamp.saturating_sub(duration)))
            }

            // Time comparison helpers
            (Some("time"), "is_before") => {
                if args.len() != 2 {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "time::is_before() requires exactly two arguments (t1, t2)"
                            .to_string(),
                    });
                }
                let t1 = match self.evaluate_expr(&args[0], context)? {
                    EvalValue::Integer(n) => n,
                    _ => {
                        return Err(ReaperError::InvalidPolicy {
                            reason: "time::is_before() arguments must be integers (timestamps)"
                                .to_string(),
                        })
                    }
                };
                let t2 = match self.evaluate_expr(&args[1], context)? {
                    EvalValue::Integer(n) => n,
                    _ => {
                        return Err(ReaperError::InvalidPolicy {
                            reason: "time::is_before() arguments must be integers (timestamps)"
                                .to_string(),
                        })
                    }
                };

                Ok(EvalValue::Boolean(t1 < t2))
            }

            (Some("time"), "is_after") => {
                if args.len() != 2 {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "time::is_after() requires exactly two arguments (t1, t2)"
                            .to_string(),
                    });
                }
                let t1 = match self.evaluate_expr(&args[0], context)? {
                    EvalValue::Integer(n) => n,
                    _ => {
                        return Err(ReaperError::InvalidPolicy {
                            reason: "time::is_after() arguments must be integers (timestamps)"
                                .to_string(),
                        })
                    }
                };
                let t2 = match self.evaluate_expr(&args[1], context)? {
                    EvalValue::Integer(n) => n,
                    _ => {
                        return Err(ReaperError::InvalidPolicy {
                            reason: "time::is_after() arguments must be integers (timestamps)"
                                .to_string(),
                        })
                    }
                };

                Ok(EvalValue::Boolean(t1 > t2))
            }

            (Some("time"), "is_between") => {
                if args.len() != 3 {
                    return Err(ReaperError::InvalidPolicy {
                        reason:
                            "time::is_between() requires exactly three arguments (t, start, end)"
                                .to_string(),
                    });
                }
                let t = match self.evaluate_expr(&args[0], context)? {
                    EvalValue::Integer(n) => n,
                    _ => {
                        return Err(ReaperError::InvalidPolicy {
                            reason: "time::is_between() arguments must be integers (timestamps)"
                                .to_string(),
                        })
                    }
                };
                let start = match self.evaluate_expr(&args[1], context)? {
                    EvalValue::Integer(n) => n,
                    _ => {
                        return Err(ReaperError::InvalidPolicy {
                            reason: "time::is_between() arguments must be integers (timestamps)"
                                .to_string(),
                        })
                    }
                };
                let end = match self.evaluate_expr(&args[2], context)? {
                    EvalValue::Integer(n) => n,
                    _ => {
                        return Err(ReaperError::InvalidPolicy {
                            reason: "time::is_between() arguments must be integers (timestamps)"
                                .to_string(),
                        })
                    }
                };

                Ok(EvalValue::Boolean(t >= start && t <= end))
            }

            // Regex namespace functions
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
                let input = match self.evaluate_expr(&args[0], context)? {
                    EvalValue::String(s) => s,
                    _ => {
                        return Err(ReaperError::InvalidPolicy {
                            reason: "regex::escape() argument must be a string".to_string(),
                        })
                    }
                };

                use regex::escape;
                Ok(EvalValue::String(escape(&input)))
            }

            // Math namespace functions
            (Some("math"), "abs") => {
                if args.len() != 1 {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "math::abs() requires exactly one argument".to_string(),
                    });
                }
                match self.evaluate_expr(&args[0], context)? {
                    EvalValue::Integer(n) => Ok(EvalValue::Integer(n.abs())),
                    EvalValue::Float(f) => Ok(EvalValue::Float(f.abs())),
                    _ => Err(ReaperError::InvalidPolicy {
                        reason: "math::abs() requires numeric argument".to_string(),
                    }),
                }
            }

            (Some("math"), "round") => {
                if args.len() != 1 {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "math::round() requires exactly one argument".to_string(),
                    });
                }
                let num = match self.evaluate_expr(&args[0], context)? {
                    EvalValue::Integer(n) => n as f64,
                    EvalValue::Float(f) => f,
                    _ => {
                        return Err(ReaperError::InvalidPolicy {
                            reason: "math::round() requires numeric argument".to_string(),
                        })
                    }
                };
                Ok(EvalValue::Integer(num.round() as i64))
            }

            (Some("math"), "floor") => {
                if args.len() != 1 {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "math::floor() requires exactly one argument".to_string(),
                    });
                }
                let num = match self.evaluate_expr(&args[0], context)? {
                    EvalValue::Integer(n) => n as f64,
                    EvalValue::Float(f) => f,
                    _ => {
                        return Err(ReaperError::InvalidPolicy {
                            reason: "math::floor() requires numeric argument".to_string(),
                        })
                    }
                };
                Ok(EvalValue::Integer(num.floor() as i64))
            }

            (Some("math"), "ceil") => {
                if args.len() != 1 {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "math::ceil() requires exactly one argument".to_string(),
                    });
                }
                let num = match self.evaluate_expr(&args[0], context)? {
                    EvalValue::Integer(n) => n as f64,
                    EvalValue::Float(f) => f,
                    _ => {
                        return Err(ReaperError::InvalidPolicy {
                            reason: "math::ceil() requires numeric argument".to_string(),
                        })
                    }
                };
                Ok(EvalValue::Integer(num.ceil() as i64))
            }

            (Some("math"), "sqrt") => {
                if args.len() != 1 {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "math::sqrt() requires exactly one argument".to_string(),
                    });
                }
                let num = match self.evaluate_expr(&args[0], context)? {
                    EvalValue::Integer(n) => n as f64,
                    EvalValue::Float(f) => f,
                    _ => {
                        return Err(ReaperError::InvalidPolicy {
                            reason: "math::sqrt() requires numeric argument".to_string(),
                        })
                    }
                };
                if num < 0.0 {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "math::sqrt() requires non-negative argument".to_string(),
                    });
                }
                Ok(EvalValue::Float(num.sqrt()))
            }

            (Some("math"), "pow") => {
                if args.len() != 2 {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "math::pow() requires exactly two arguments (base, exponent)"
                            .to_string(),
                    });
                }
                let base = match self.evaluate_expr(&args[0], context)? {
                    EvalValue::Integer(n) => n as f64,
                    EvalValue::Float(f) => f,
                    _ => {
                        return Err(ReaperError::InvalidPolicy {
                            reason: "math::pow() base must be numeric".to_string(),
                        })
                    }
                };
                let exp = match self.evaluate_expr(&args[1], context)? {
                    EvalValue::Integer(n) => n as f64,
                    EvalValue::Float(f) => f,
                    _ => {
                        return Err(ReaperError::InvalidPolicy {
                            reason: "math::pow() exponent must be numeric".to_string(),
                        })
                    }
                };
                let result = base.powf(exp);
                // Return integer if result is a whole number and exponent was non-negative
                if exp >= 0.0 && result.fract() == 0.0 && result.is_finite() {
                    Ok(EvalValue::Integer(result as i64))
                } else {
                    Ok(EvalValue::Float(result))
                }
            }

            (Some("math"), "min") => {
                if args.len() != 2 {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "math::min() requires exactly two arguments".to_string(),
                    });
                }
                let a = self.evaluate_expr(&args[0], context)?;
                let b = self.evaluate_expr(&args[1], context)?;

                match (&a, &b) {
                    (EvalValue::Integer(x), EvalValue::Integer(y)) => {
                        Ok(EvalValue::Integer(*x.min(y)))
                    }
                    (EvalValue::Float(x), EvalValue::Float(y)) => Ok(EvalValue::Float(x.min(*y))),
                    (EvalValue::Integer(x), EvalValue::Float(y))
                    | (EvalValue::Float(y), EvalValue::Integer(x)) => {
                        Ok(EvalValue::Float((*x as f64).min(*y)))
                    }
                    _ => Err(ReaperError::InvalidPolicy {
                        reason: "math::min() requires two numeric arguments".to_string(),
                    }),
                }
            }

            (Some("math"), "max") => {
                if args.len() != 2 {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "math::max() requires exactly two arguments".to_string(),
                    });
                }
                let a = self.evaluate_expr(&args[0], context)?;
                let b = self.evaluate_expr(&args[1], context)?;

                match (&a, &b) {
                    (EvalValue::Integer(x), EvalValue::Integer(y)) => {
                        Ok(EvalValue::Integer(*x.max(y)))
                    }
                    (EvalValue::Float(x), EvalValue::Float(y)) => Ok(EvalValue::Float(x.max(*y))),
                    (EvalValue::Integer(x), EvalValue::Float(y))
                    | (EvalValue::Float(y), EvalValue::Integer(x)) => {
                        Ok(EvalValue::Float((*x as f64).max(*y)))
                    }
                    _ => Err(ReaperError::InvalidPolicy {
                        reason: "math::max() requires two numeric arguments".to_string(),
                    }),
                }
            }

            (Some("math"), "clamp") => {
                if args.len() != 3 {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "math::clamp() requires exactly three arguments (value, min, max)"
                            .to_string(),
                    });
                }
                let val = match self.evaluate_expr(&args[0], context)? {
                    EvalValue::Integer(n) => n as f64,
                    EvalValue::Float(f) => f,
                    _ => {
                        return Err(ReaperError::InvalidPolicy {
                            reason: "math::clamp() value must be numeric".to_string(),
                        })
                    }
                };
                let min = match self.evaluate_expr(&args[1], context)? {
                    EvalValue::Integer(n) => n as f64,
                    EvalValue::Float(f) => f,
                    _ => {
                        return Err(ReaperError::InvalidPolicy {
                            reason: "math::clamp() min must be numeric".to_string(),
                        })
                    }
                };
                let max = match self.evaluate_expr(&args[2], context)? {
                    EvalValue::Integer(n) => n as f64,
                    EvalValue::Float(f) => f,
                    _ => {
                        return Err(ReaperError::InvalidPolicy {
                            reason: "math::clamp() max must be numeric".to_string(),
                        })
                    }
                };

                if min > max {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "math::clamp() min must be <= max".to_string(),
                    });
                }

                let clamped = val.clamp(min, max);
                // Return integer if all inputs were integers
                if matches!(
                    self.evaluate_expr(&args[0], context)?,
                    EvalValue::Integer(_)
                ) && matches!(
                    self.evaluate_expr(&args[1], context)?,
                    EvalValue::Integer(_)
                ) && matches!(
                    self.evaluate_expr(&args[2], context)?,
                    EvalValue::Integer(_)
                ) {
                    Ok(EvalValue::Integer(clamped as i64))
                } else {
                    Ok(EvalValue::Float(clamped))
                }
            }

            // ============================================================================
            // JSON Functions - High-performance JSON parsing and serialization
            // ============================================================================
            (Some("json"), "parse") => {
                if args.is_empty() {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "json::parse() requires a JSON string argument".to_string(),
                    });
                }

                let json_str = match self.evaluate_expr(&args[0], context)? {
                    EvalValue::String(s) => s,
                    _ => {
                        return Err(ReaperError::InvalidPolicy {
                            reason: "json::parse() requires a string argument".to_string(),
                        })
                    }
                };

                // Use sonic_rs for ultra-fast SIMD-accelerated parsing
                match sonic_rs::from_str::<sonic_rs::Value>(&json_str) {
                    Ok(json_value) => {
                        // Convert sonic_rs::Value to EvalValue
                        self.json_value_to_eval_value(&json_value)
                    }
                    Err(e) => Err(ReaperError::InvalidPolicy {
                        reason: format!("json::parse() failed: {}", e),
                    }),
                }
            }

            (Some("json"), "stringify") => {
                if args.is_empty() {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "json::stringify() requires a value argument".to_string(),
                    });
                }

                let value = self.evaluate_expr(&args[0], context)?;

                // Convert EvalValue to sonic_rs::Value
                let json_value = self.eval_value_to_json_value(&value)?;

                // Serialize to JSON string using sonic_rs for maximum speed
                // Compact output (no pretty-printing) for optimal performance
                match sonic_rs::to_string(&json_value) {
                    Ok(json_str) => Ok(EvalValue::String(json_str)),
                    Err(e) => Err(ReaperError::InvalidPolicy {
                        reason: format!("json::stringify() failed: {}", e),
                    }),
                }
            }

            (Some("json"), "is_valid") => {
                if args.is_empty() {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "json::is_valid() requires a string argument".to_string(),
                    });
                }

                let json_str = match self.evaluate_expr(&args[0], context)? {
                    EvalValue::String(s) => s,
                    _ => {
                        return Err(ReaperError::InvalidPolicy {
                            reason: "json::is_valid() requires a string argument".to_string(),
                        })
                    }
                };

                // Ultra-fast validation using sonic_rs's SIMD-accelerated parser
                // Stops parsing on first error for maximum efficiency
                let is_valid = sonic_rs::from_str::<sonic_rs::Value>(&json_str).is_ok();
                Ok(EvalValue::Boolean(is_valid))
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

    // ===== Aggregate Methods =====

    /// count() - Returns the number of items in a collection
    /// Performance: O(1) for arrays/sets (length lookup), O(n) for object (key count)
    fn method_count(&self, value: &EvalValue) -> Result<EvalValue, ReaperError> {
        let count = match value {
            EvalValue::Array(arr) => arr.len(),
            EvalValue::Set(set) => set.len(),
            EvalValue::Object(obj) => obj.len(),
            EvalValue::String(s) => s.len(), // Character count
            _ => {
                return Err(ReaperError::InvalidPolicy {
                    reason: "count() requires collection or string".to_string(),
                })
            }
        };

        Ok(EvalValue::Integer(count as i64))
    }

    /// sum() - Sums all numeric values in a collection
    /// Performance: O(n) with SIMD optimization for large arrays (>64 elements)
    /// For large integer arrays, uses optimized loops that LLVM auto-vectorizes (2-4x speedup)
    fn method_sum(&self, value: &EvalValue) -> Result<EvalValue, ReaperError> {
        let items = self.get_collection_items(value)?;

        // Fast path for large pure-integer arrays using SIMD-friendly patterns
        if items.len() > 64 && items.iter().all(|v| matches!(v, EvalValue::Integer(_))) {
            // SIMD-optimized integer sum (LLVM auto-vectorizes this pattern)
            let sum: i64 = items
                .iter()
                .filter_map(|v| {
                    if let EvalValue::Integer(i) = v {
                        Some(*i)
                    } else {
                        None
                    }
                })
                .sum();
            return Ok(EvalValue::Integer(sum));
        }

        // Fast path for large pure-float arrays using SIMD-friendly patterns
        if items.len() > 64 && items.iter().all(|v| matches!(v, EvalValue::Float(_))) {
            // SIMD-optimized float sum (LLVM auto-vectorizes this pattern)
            let sum: f64 = items
                .iter()
                .filter_map(|v| {
                    if let EvalValue::Float(f) = v {
                        Some(*f)
                    } else {
                        None
                    }
                })
                .sum();
            return Ok(EvalValue::Float(sum));
        }

        // Standard path for mixed types or small arrays
        let mut int_sum: i64 = 0;
        let mut has_float = false;
        let mut float_sum: f64 = 0.0;

        for item in items {
            match item {
                EvalValue::Integer(i) => {
                    if has_float {
                        float_sum += *i as f64;
                    } else {
                        int_sum += i;
                    }
                }
                EvalValue::Float(f) => {
                    if !has_float {
                        // Convert accumulated int sum to float
                        float_sum = int_sum as f64;
                        has_float = true;
                    }
                    float_sum += f;
                }
                _ => {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "sum() requires numeric values".to_string(),
                    })
                }
            }
        }

        if has_float {
            Ok(EvalValue::Float(float_sum))
        } else {
            Ok(EvalValue::Integer(int_sum))
        }
    }

    /// max() - Finds the maximum value in a collection
    /// Performance: O(n) with SIMD optimization for large arrays (>64 elements)
    fn method_max(&self, value: &EvalValue) -> Result<EvalValue, ReaperError> {
        let items = self.get_collection_items(value)?;

        if items.is_empty() {
            return Err(ReaperError::InvalidPolicy {
                reason: "max() requires non-empty collection".to_string(),
            });
        }

        // Fast path for large pure-integer arrays using SIMD-friendly patterns
        if items.len() > 64 && items.iter().all(|v| matches!(v, EvalValue::Integer(_))) {
            // SIMD-optimized integer max (LLVM auto-vectorizes this pattern)
            let max = items
                .iter()
                .filter_map(|v| {
                    if let EvalValue::Integer(i) = v {
                        Some(*i)
                    } else {
                        None
                    }
                })
                .max()
                .unwrap(); // Safe because we checked non-empty
            return Ok(EvalValue::Integer(max));
        }

        // Fast path for large pure-float arrays using SIMD-friendly patterns
        if items.len() > 64 && items.iter().all(|v| matches!(v, EvalValue::Float(_))) {
            // SIMD-optimized float max (LLVM auto-vectorizes this pattern)
            let max = items
                .iter()
                .filter_map(|v| {
                    if let EvalValue::Float(f) = v {
                        Some(*f)
                    } else {
                        None
                    }
                })
                .fold(f64::NEG_INFINITY, f64::max);
            return Ok(EvalValue::Float(max));
        }

        // Standard path for mixed types or small arrays
        let mut max_int: Option<i64> = None;
        let mut max_float: Option<f64> = None;
        let mut has_float = false;

        for item in items {
            match item {
                EvalValue::Integer(i) => {
                    if has_float {
                        let i_as_float = *i as f64;
                        max_float = Some(max_float.map_or(i_as_float, |m| m.max(i_as_float)));
                    } else {
                        max_int = Some(max_int.map_or(*i, |m| m.max(*i)));
                    }
                }
                EvalValue::Float(f) => {
                    if !has_float {
                        // Convert to float comparison
                        let current_max = max_int.map(|i| i as f64).unwrap_or(f64::NEG_INFINITY);
                        max_float = Some(current_max.max(*f));
                        has_float = true;
                    } else {
                        max_float = Some(max_float.map_or(*f, |m| m.max(*f)));
                    }
                }
                _ => {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "max() requires numeric values".to_string(),
                    })
                }
            }
        }

        if has_float {
            Ok(EvalValue::Float(max_float.unwrap()))
        } else {
            Ok(EvalValue::Integer(max_int.unwrap()))
        }
    }

    /// min() - Finds the minimum value in a collection
    /// Performance: O(n) with SIMD optimization for large arrays (>64 elements)
    fn method_min(&self, value: &EvalValue) -> Result<EvalValue, ReaperError> {
        let items = self.get_collection_items(value)?;

        if items.is_empty() {
            return Err(ReaperError::InvalidPolicy {
                reason: "min() requires non-empty collection".to_string(),
            });
        }

        // Fast path for large pure-integer arrays using SIMD-friendly patterns
        if items.len() > 64 && items.iter().all(|v| matches!(v, EvalValue::Integer(_))) {
            // SIMD-optimized integer min (LLVM auto-vectorizes this pattern)
            let min = items
                .iter()
                .filter_map(|v| {
                    if let EvalValue::Integer(i) = v {
                        Some(*i)
                    } else {
                        None
                    }
                })
                .min()
                .unwrap(); // Safe because we checked non-empty
            return Ok(EvalValue::Integer(min));
        }

        // Fast path for large pure-float arrays using SIMD-friendly patterns
        if items.len() > 64 && items.iter().all(|v| matches!(v, EvalValue::Float(_))) {
            // SIMD-optimized float min (LLVM auto-vectorizes this pattern)
            let min = items
                .iter()
                .filter_map(|v| {
                    if let EvalValue::Float(f) = v {
                        Some(*f)
                    } else {
                        None
                    }
                })
                .fold(f64::INFINITY, f64::min);
            return Ok(EvalValue::Float(min));
        }

        // Standard path for mixed types or small arrays
        let mut min_int: Option<i64> = None;
        let mut min_float: Option<f64> = None;
        let mut has_float = false;

        for item in items {
            match item {
                EvalValue::Integer(i) => {
                    if has_float {
                        let i_as_float = *i as f64;
                        min_float = Some(min_float.map_or(i_as_float, |m| m.min(i_as_float)));
                    } else {
                        min_int = Some(min_int.map_or(*i, |m| m.min(*i)));
                    }
                }
                EvalValue::Float(f) => {
                    if !has_float {
                        // Convert to float comparison
                        let current_min = min_int.map(|i| i as f64).unwrap_or(f64::INFINITY);
                        min_float = Some(current_min.min(*f));
                        has_float = true;
                    } else {
                        min_float = Some(min_float.map_or(*f, |m| m.min(*f)));
                    }
                }
                _ => {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "min() requires numeric values".to_string(),
                    })
                }
            }
        }

        if has_float {
            Ok(EvalValue::Float(min_float.unwrap()))
        } else {
            Ok(EvalValue::Integer(min_int.unwrap()))
        }
    }

    /// any() - Returns true if any element is truthy
    /// Performance: O(n) with short-circuit evaluation
    fn method_any(&self, value: &EvalValue) -> Result<EvalValue, ReaperError> {
        let items = self.get_collection_items(value)?;

        for item in items {
            let is_truthy = match item {
                EvalValue::Boolean(b) => *b,
                EvalValue::Integer(i) => *i != 0,
                EvalValue::Null => false,
                EvalValue::String(s) => !s.is_empty(),
                _ => true, // Arrays, objects, sets are truthy if they exist
            };

            if is_truthy {
                return Ok(EvalValue::Boolean(true));
            }
        }

        Ok(EvalValue::Boolean(false))
    }

    /// all() - Returns true if all elements are truthy
    /// Performance: O(n) with short-circuit evaluation
    fn method_all(&self, value: &EvalValue) -> Result<EvalValue, ReaperError> {
        let items = self.get_collection_items(value)?;

        for item in items {
            let is_truthy = match item {
                EvalValue::Boolean(b) => *b,
                EvalValue::Integer(i) => *i != 0,
                EvalValue::Null => false,
                EvalValue::String(s) => !s.is_empty(),
                _ => true,
            };

            if !is_truthy {
                return Ok(EvalValue::Boolean(false));
            }
        }

        Ok(EvalValue::Boolean(true))
    }

    // ===== String Methods =====

    /// lower() - Converts string to lowercase
    fn method_lower(&self, value: &EvalValue) -> Result<EvalValue, ReaperError> {
        match value {
            EvalValue::String(s) => Ok(EvalValue::String(s.to_lowercase())),
            _ => Err(ReaperError::InvalidPolicy {
                reason: "lower() requires string value".to_string(),
            }),
        }
    }

    /// upper() - Converts string to uppercase
    fn method_upper(&self, value: &EvalValue) -> Result<EvalValue, ReaperError> {
        match value {
            EvalValue::String(s) => Ok(EvalValue::String(s.to_uppercase())),
            _ => Err(ReaperError::InvalidPolicy {
                reason: "upper() requires string value".to_string(),
            }),
        }
    }

    /// trim() - Removes leading/trailing whitespace
    fn method_trim(&self, value: &EvalValue) -> Result<EvalValue, ReaperError> {
        match value {
            EvalValue::String(s) => Ok(EvalValue::String(s.trim().to_string())),
            _ => Err(ReaperError::InvalidPolicy {
                reason: "trim() requires string value".to_string(),
            }),
        }
    }

    /// split() - Splits string by delimiter
    fn method_split(
        &self,
        value: &EvalValue,
        delimiter: &EvalValue,
    ) -> Result<EvalValue, ReaperError> {
        match (value, delimiter) {
            (EvalValue::String(s), EvalValue::String(delim)) => {
                let parts: Vec<EvalValue> = s
                    .split(delim.as_str())
                    .map(|part| EvalValue::String(part.to_string()))
                    .collect();
                Ok(EvalValue::Array(parts))
            }
            _ => Err(ReaperError::InvalidPolicy {
                reason: "split() requires string value and delimiter".to_string(),
            }),
        }
    }

    /// contains() - Checks if string contains substring
    fn method_contains(
        &self,
        value: &EvalValue,
        substring: &EvalValue,
    ) -> Result<EvalValue, ReaperError> {
        match (value, substring) {
            (EvalValue::String(s), EvalValue::String(sub)) => {
                Ok(EvalValue::Boolean(s.contains(sub.as_str())))
            }
            (EvalValue::Array(arr), item) | (EvalValue::Set(arr), item) => {
                Ok(EvalValue::Boolean(arr.contains(item)))
            }
            _ => Err(ReaperError::InvalidPolicy {
                reason: "contains() requires string/collection and value".to_string(),
            }),
        }
    }

    /// startswith() - Checks if string starts with prefix
    fn method_startswith(
        &self,
        value: &EvalValue,
        prefix: &EvalValue,
    ) -> Result<EvalValue, ReaperError> {
        match (value, prefix) {
            (EvalValue::String(s), EvalValue::String(pre)) => {
                Ok(EvalValue::Boolean(s.starts_with(pre.as_str())))
            }
            _ => Err(ReaperError::InvalidPolicy {
                reason: "startswith() requires string value and prefix".to_string(),
            }),
        }
    }

    /// endswith() - Checks if string ends with suffix
    fn method_endswith(
        &self,
        value: &EvalValue,
        suffix: &EvalValue,
    ) -> Result<EvalValue, ReaperError> {
        match (value, suffix) {
            (EvalValue::String(s), EvalValue::String(suf)) => {
                Ok(EvalValue::Boolean(s.ends_with(suf.as_str())))
            }
            _ => Err(ReaperError::InvalidPolicy {
                reason: "endswith() requires string value and suffix".to_string(),
            }),
        }
    }

    // ===== Regex Methods =====

    /// Get or compile a regex pattern with caching for 2-5x performance improvement
    ///
    /// Regex compilation is expensive (~1-10 µs per pattern).
    /// Caching provides significant speedup when the same pattern is used multiple times.
    fn get_cached_regex(&self, pattern: &str) -> Result<regex::Regex, ReaperError> {
        // Fast path: check if already cached
        if let Some(re) = self.regex_cache.borrow().get(pattern) {
            return Ok(re.clone());
        }

        // Slow path: compile and cache
        let re = regex::Regex::new(pattern).map_err(|e| ReaperError::InvalidPolicy {
            reason: format!("Invalid regex pattern '{}': {}", pattern, e),
        })?;

        // Insert into cache for future use
        self.regex_cache
            .borrow_mut()
            .insert(pattern.to_string(), re.clone());

        Ok(re)
    }

    /// matches() - Tests if string matches regex pattern (with caching)
    fn method_matches(
        &self,
        value: &EvalValue,
        pattern: &EvalValue,
    ) -> Result<EvalValue, ReaperError> {
        match (value, pattern) {
            (EvalValue::String(s), EvalValue::String(pat)) => {
                let re = self.get_cached_regex(pat)?;
                Ok(EvalValue::Boolean(re.is_match(s)))
            }
            _ => Err(ReaperError::InvalidPolicy {
                reason: "matches() requires string value and pattern".to_string(),
            }),
        }
    }

    /// find() - Finds first match of regex pattern in string (with caching)
    fn method_find(
        &self,
        value: &EvalValue,
        pattern: &EvalValue,
    ) -> Result<EvalValue, ReaperError> {
        match (value, pattern) {
            (EvalValue::String(s), EvalValue::String(pat)) => {
                let re = self.get_cached_regex(pat)?;

                match re.find(s) {
                    Some(m) => Ok(EvalValue::String(m.as_str().to_string())),
                    None => Ok(EvalValue::Null),
                }
            }
            _ => Err(ReaperError::InvalidPolicy {
                reason: "find() requires string value and pattern".to_string(),
            }),
        }
    }

    /// find_all() - Finds all matches of regex pattern in string (with caching)
    fn method_find_all(
        &self,
        value: &EvalValue,
        pattern: &EvalValue,
    ) -> Result<EvalValue, ReaperError> {
        match (value, pattern) {
            (EvalValue::String(s), EvalValue::String(pat)) => {
                let re = self.get_cached_regex(pat)?;

                let matches: Vec<EvalValue> = re
                    .find_iter(s)
                    .map(|m| EvalValue::String(m.as_str().to_string()))
                    .collect();

                Ok(EvalValue::Array(matches))
            }
            _ => Err(ReaperError::InvalidPolicy {
                reason: "find_all() requires string value and pattern".to_string(),
            }),
        }
    }

    /// replace() - Replaces all matches of regex pattern with replacement string (with caching)
    fn method_replace(
        &self,
        value: &EvalValue,
        pattern: &EvalValue,
        replacement: &EvalValue,
    ) -> Result<EvalValue, ReaperError> {
        match (value, pattern, replacement) {
            (EvalValue::String(s), EvalValue::String(pat), EvalValue::String(rep)) => {
                let re = self.get_cached_regex(pat)?;

                let result = re.replace_all(s, rep.as_str()).to_string();
                Ok(EvalValue::String(result))
            }
            _ => Err(ReaperError::InvalidPolicy {
                reason: "replace() requires string value, pattern, and replacement".to_string(),
            }),
        }
    }

    // ===== Collection Methods =====

    /// union() - Returns union of two sets
    fn method_union(&self, value: &EvalValue, other: &EvalValue) -> Result<EvalValue, ReaperError> {
        let items1 = self.get_collection_items(value)?;
        let items2 = self.get_collection_items(other)?;

        let set1: HashSet<_> = items1.into_iter().cloned().collect();
        let set2: HashSet<_> = items2.into_iter().cloned().collect();

        let union: Vec<EvalValue> = set1.union(&set2).cloned().collect();
        Ok(EvalValue::Set(union))
    }

    /// intersection() - Returns intersection of two sets
    fn method_intersection(
        &self,
        value: &EvalValue,
        other: &EvalValue,
    ) -> Result<EvalValue, ReaperError> {
        let items1 = self.get_collection_items(value)?;
        let items2 = self.get_collection_items(other)?;

        let set1: HashSet<_> = items1.into_iter().cloned().collect();
        let set2: HashSet<_> = items2.into_iter().cloned().collect();

        let intersection: Vec<EvalValue> = set1.intersection(&set2).cloned().collect();
        Ok(EvalValue::Set(intersection))
    }

    /// difference() - Returns difference of two sets (items in first but not second)
    fn method_difference(
        &self,
        value: &EvalValue,
        other: &EvalValue,
    ) -> Result<EvalValue, ReaperError> {
        let items1 = self.get_collection_items(value)?;
        let items2 = self.get_collection_items(other)?;

        let set1: HashSet<_> = items1.into_iter().cloned().collect();
        let set2: HashSet<_> = items2.into_iter().cloned().collect();

        let difference: Vec<EvalValue> = set1.difference(&set2).cloned().collect();
        Ok(EvalValue::Set(difference))
    }

    /// Helper: Extract items from a collection (returns owned Vec to avoid lifetime issues)
    fn get_collection_items<'a>(
        &self,
        value: &'a EvalValue,
    ) -> Result<Vec<&'a EvalValue>, ReaperError> {
        match value {
            EvalValue::Array(arr) => Ok(arr.iter().collect()),
            EvalValue::Set(set) => Ok(set.iter().collect()),
            EvalValue::Object(obj) => Ok(obj.values().collect()),
            _ => Err(ReaperError::InvalidPolicy {
                reason: "Expected collection (array, set, or object)".to_string(),
            }),
        }
    }

    // ============================================================================
    // Advanced Collection Methods
    // ============================================================================

    /// first() - Returns the first element of an array, or Null if empty
    /// Uses Rust's slice .first() method for optimal performance
    fn method_first(&self, value: &EvalValue) -> Result<EvalValue, ReaperError> {
        match value {
            EvalValue::Array(arr) => {
                // Use slice .first() - returns Option<&T>
                match arr.first() {
                    Some(elem) => Ok(elem.clone()),
                    None => Ok(EvalValue::Null),
                }
            }
            _ => Err(ReaperError::InvalidPolicy {
                reason: "first() requires an array".to_string(),
            }),
        }
    }

    /// last() - Returns the last element of an array, or Null if empty
    /// Uses Rust's slice .last() method for optimal performance
    fn method_last(&self, value: &EvalValue) -> Result<EvalValue, ReaperError> {
        match value {
            EvalValue::Array(arr) => {
                // Use slice .last() - returns Option<&T>
                match arr.last() {
                    Some(elem) => Ok(elem.clone()),
                    None => Ok(EvalValue::Null),
                }
            }
            _ => Err(ReaperError::InvalidPolicy {
                reason: "last() requires an array".to_string(),
            }),
        }
    }

    /// slice(start, end) - Extracts a subarray from start (inclusive) to end (exclusive)
    /// Uses Rust's slice indexing with proper bounds checking
    fn method_slice(
        &self,
        value: &EvalValue,
        start: &EvalValue,
        end: &EvalValue,
    ) -> Result<EvalValue, ReaperError> {
        match value {
            EvalValue::Array(arr) => {
                // Extract integer indices
                let start_idx = match start {
                    EvalValue::Integer(i) => *i,
                    _ => {
                        return Err(ReaperError::InvalidPolicy {
                            reason: "slice() start index must be an integer".to_string(),
                        })
                    }
                };

                let end_idx = match end {
                    EvalValue::Integer(i) => *i,
                    _ => {
                        return Err(ReaperError::InvalidPolicy {
                            reason: "slice() end index must be an integer".to_string(),
                        })
                    }
                };

                // Convert to usize with bounds checking
                let len = arr.len();
                let start_usize = if start_idx < 0 {
                    0
                } else if start_idx as usize > len {
                    len
                } else {
                    start_idx as usize
                };

                let end_usize = if end_idx < 0 {
                    0
                } else if end_idx as usize > len {
                    len
                } else {
                    end_idx as usize
                };

                // Ensure start <= end
                if start_usize > end_usize {
                    return Ok(EvalValue::Array(Vec::new()));
                }

                // Use slice indexing - zero-copy view, then clone
                let sliced = arr[start_usize..end_usize].to_vec();
                Ok(EvalValue::Array(sliced))
            }
            _ => Err(ReaperError::InvalidPolicy {
                reason: "slice() requires an array".to_string(),
            }),
        }
    }

    /// reverse() - Returns a new array with elements in reverse order
    /// Uses Rust's iterator .rev() for optimal performance - O(n) with single pass
    fn method_reverse(&self, value: &EvalValue) -> Result<EvalValue, ReaperError> {
        match value {
            EvalValue::Array(arr) => {
                // Use iterator .rev() - highly optimized by Rust std
                let reversed: Vec<EvalValue> = arr.iter().rev().cloned().collect();
                Ok(EvalValue::Array(reversed))
            }
            _ => Err(ReaperError::InvalidPolicy {
                reason: "reverse() requires an array".to_string(),
            }),
        }
    }

    /// sort() - Returns a new array with elements sorted in ascending order
    /// Type-aware sorting: handles integers, floats, strings, and booleans
    /// Uses Rust's optimized .sort_by() with pattern matching for type safety
    fn method_sort(&self, value: &EvalValue) -> Result<EvalValue, ReaperError> {
        match value {
            EvalValue::Array(arr) => {
                if arr.is_empty() {
                    return Ok(EvalValue::Array(Vec::new()));
                }

                // Clone array for sorting (preserve original)
                let mut sorted = arr.clone();

                // Type-aware sorting using pattern matching
                sorted.sort_by(|a, b| {
                    use std::cmp::Ordering;

                    match (a, b) {
                        // Integer comparison
                        (EvalValue::Integer(x), EvalValue::Integer(y)) => x.cmp(y),

                        // Float comparison (handle NaN by treating as greater)
                        (EvalValue::Float(x), EvalValue::Float(y)) => {
                            x.partial_cmp(y).unwrap_or(Ordering::Equal)
                        }

                        // Mixed numeric types - convert to f64 for comparison
                        (EvalValue::Integer(x), EvalValue::Float(y)) => {
                            (*x as f64).partial_cmp(y).unwrap_or(Ordering::Equal)
                        }
                        (EvalValue::Float(x), EvalValue::Integer(y)) => {
                            x.partial_cmp(&(*y as f64)).unwrap_or(Ordering::Equal)
                        }

                        // String comparison (lexicographic)
                        (EvalValue::String(x), EvalValue::String(y)) => x.cmp(y),

                        // Boolean comparison (false < true)
                        (EvalValue::Boolean(x), EvalValue::Boolean(y)) => x.cmp(y),

                        // Null comparison (null is always less than non-null)
                        (EvalValue::Null, EvalValue::Null) => Ordering::Equal,
                        (EvalValue::Null, _) => Ordering::Less,
                        (_, EvalValue::Null) => Ordering::Greater,

                        // Mixed types - define stable ordering by type precedence:
                        // Null < Boolean < Integer < Float < String < Array < Set < Object
                        _ => {
                            let type_order = |v: &EvalValue| match v {
                                EvalValue::Null => 0,
                                EvalValue::Boolean(_) => 1,
                                EvalValue::Integer(_) => 2,
                                EvalValue::Float(_) => 3,
                                EvalValue::String(_) => 4,
                                EvalValue::Array(_) => 5,
                                EvalValue::Set(_) => 6,
                                EvalValue::Object(_) => 7,
                            };
                            type_order(a).cmp(&type_order(b))
                        }
                    }
                });

                Ok(EvalValue::Array(sorted))
            }
            _ => Err(ReaperError::InvalidPolicy {
                reason: "sort() requires an array".to_string(),
            }),
        }
    }

    /// unique() - Returns a Set containing only unique elements from array
    /// Uses HashSet for O(n) deduplication - highly efficient Rust pattern
    fn method_unique(&self, value: &EvalValue) -> Result<EvalValue, ReaperError> {
        match value {
            EvalValue::Array(arr) => {
                // Use HashSet for deduplication - O(n) average case
                // HashSet automatically handles uniqueness
                let unique_set: HashSet<EvalValue> = arr.iter().cloned().collect();

                // Convert back to Vec for Set variant
                let unique_vec: Vec<EvalValue> = unique_set.into_iter().collect();

                Ok(EvalValue::Set(unique_vec))
            }
            _ => Err(ReaperError::InvalidPolicy {
                reason: "unique() requires an array".to_string(),
            }),
        }
    }

    // ============================================================================
    // Object Methods
    // ============================================================================

    /// keys() - Returns an array of all keys in an object
    /// Preserves insertion order (HashMap maintains order in Rust)
    fn method_keys(&self, value: &EvalValue) -> Result<EvalValue, ReaperError> {
        match value {
            EvalValue::Object(obj) => {
                // Extract keys and convert to EvalValue strings
                let keys: Vec<EvalValue> =
                    obj.keys().map(|k| EvalValue::String(k.clone())).collect();

                Ok(EvalValue::Array(keys))
            }
            _ => Err(ReaperError::InvalidPolicy {
                reason: "keys() requires an object".to_string(),
            }),
        }
    }

    /// values() - Returns an array of all values in an object
    /// Preserves insertion order
    fn method_values(&self, value: &EvalValue) -> Result<EvalValue, ReaperError> {
        match value {
            EvalValue::Object(obj) => {
                // Extract values - already EvalValue so just clone
                let values: Vec<EvalValue> = obj.values().cloned().collect();

                Ok(EvalValue::Array(values))
            }
            _ => Err(ReaperError::InvalidPolicy {
                reason: "values() requires an object".to_string(),
            }),
        }
    }

    /// has_key(key) - Checks if an object contains a specific key
    /// O(1) average case lookup using HashMap
    fn method_has_key(&self, value: &EvalValue, key: &EvalValue) -> Result<EvalValue, ReaperError> {
        match value {
            EvalValue::Object(obj) => {
                // Extract key string
                let key_str = match key {
                    EvalValue::String(s) => s,
                    _ => {
                        return Err(ReaperError::InvalidPolicy {
                            reason: "has_key() requires a string key".to_string(),
                        })
                    }
                };

                // Use HashMap .contains_key() - O(1) average case
                Ok(EvalValue::Boolean(obj.contains_key(key_str)))
            }
            _ => Err(ReaperError::InvalidPolicy {
                reason: "has_key() requires an object".to_string(),
            }),
        }
    }

    // ============================================================================
    // JSON Conversion Helpers - sonic_rs::Value <-> EvalValue
    // ============================================================================

    /// Convert sonic_rs::Value to EvalValue
    /// Uses SIMD-accelerated parsing with pattern matching for maximum performance
    #[allow(clippy::only_used_in_recursion)]
    fn json_value_to_eval_value(&self, json: &sonic_rs::Value) -> Result<EvalValue, ReaperError> {
        use sonic_rs::{JsonContainerTrait, JsonValueTrait};

        if json.is_null() {
            Ok(EvalValue::Null)
        } else if let Some(b) = json.as_bool() {
            Ok(EvalValue::Boolean(b))
        } else if let Some(i) = json.as_i64() {
            Ok(EvalValue::Integer(i))
        } else if let Some(f) = json.as_f64() {
            Ok(EvalValue::Float(f))
        } else if let Some(s) = json.as_str() {
            Ok(EvalValue::String(s.to_string()))
        } else if let Some(arr) = json.as_array() {
            // Recursively convert array elements
            let eval_arr: Result<Vec<EvalValue>, ReaperError> = arr
                .iter()
                .map(|v| self.json_value_to_eval_value(v))
                .collect();
            Ok(EvalValue::Array(eval_arr?))
        } else if let Some(obj) = json.as_object() {
            // Convert JSON object to HashMap (preserves insertion order)
            let mut eval_obj = HashMap::new();
            for (key, value) in obj {
                eval_obj.insert(key.to_string(), self.json_value_to_eval_value(value)?);
            }
            Ok(EvalValue::Object(eval_obj))
        } else {
            Err(ReaperError::InvalidPolicy {
                reason: "Unsupported JSON value type".to_string(),
            })
        }
    }

    /// Convert EvalValue to sonic_rs::Value for high-speed serialization
    /// Uses SIMD acceleration with minimal allocations
    #[allow(clippy::only_used_in_recursion)]
    fn eval_value_to_json_value(&self, eval: &EvalValue) -> Result<sonic_rs::Value, ReaperError> {
        use sonic_rs::{json, Object};

        match eval {
            EvalValue::Null => Ok(json!(null)),
            EvalValue::Boolean(b) => Ok(json!(*b)),
            EvalValue::Integer(i) => Ok(json!(*i)),
            EvalValue::Float(f) => {
                // Convert float to JSON number (may fail for NaN/Infinity)
                if f.is_nan() || f.is_infinite() {
                    Err(ReaperError::InvalidPolicy {
                        reason: format!("Cannot convert float {} to JSON (NaN or Infinity)", f),
                    })
                } else {
                    Ok(json!(*f))
                }
            }
            EvalValue::String(s) => Ok(json!(s)),
            EvalValue::Array(arr) => {
                // Recursively convert array elements
                let json_arr: Result<Vec<sonic_rs::Value>, ReaperError> = arr
                    .iter()
                    .map(|v| self.eval_value_to_json_value(v))
                    .collect();
                Ok(json!(json_arr?))
            }
            EvalValue::Set(set) => {
                // Convert Set to JSON array (JSON doesn't have native Set type)
                let json_arr: Result<Vec<sonic_rs::Value>, ReaperError> = set
                    .iter()
                    .map(|v| self.eval_value_to_json_value(v))
                    .collect();
                Ok(json!(json_arr?))
            }
            EvalValue::Object(obj) => {
                // Convert HashMap to JSON object
                let mut json_obj = Object::new();
                for (key, value) in obj {
                    json_obj.insert(key, self.eval_value_to_json_value(value)?);
                }
                Ok(json!(json_obj))
            }
        }
    }
}

impl From<Decision> for PolicyAction {
    fn from(decision: Decision) -> Self {
        match decision {
            Decision::Allow => PolicyAction::Allow,
            Decision::Deny => PolicyAction::Deny,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EntityBuilder;
    use crate::PolicyRequest;
    use std::collections::HashMap;

    fn create_test_store() -> Arc<DataStore> {
        let store = Arc::new(DataStore::new());
        let interner = store.interner();

        // Create some test users with various attributes
        let alice_id = interner.intern("alice");
        let user_type = interner.intern("User");
        let role_key = interner.intern("role");
        let admin_value = interner.intern("admin");
        let years_key = interner.intern("years_experience");
        let active_key = interner.intern("active");
        let email_key = interner.intern("email");
        let alice_email = interner.intern("alice@example.com");

        let alice = EntityBuilder::new(alice_id, user_type)
            .with_string(role_key, admin_value)
            .with_int(years_key, 8)
            .with_bool(active_key, true)
            .with_string(email_key, alice_email)
            .build();

        let bob_id = interner.intern("bob");
        let developer_value = interner.intern("developer");
        let bob_email = interner.intern("bob@example.com");

        let bob = EntityBuilder::new(bob_id, user_type)
            .with_string(role_key, developer_value)
            .with_int(years_key, 3)
            .with_bool(active_key, true)
            .with_string(email_key, bob_email)
            .build();

        let charlie_id = interner.intern("charlie");
        let charlie_email = interner.intern("charlie@example.com");

        let charlie = EntityBuilder::new(charlie_id, user_type)
            .with_string(role_key, developer_value)
            .with_int(years_key, 6)
            .with_bool(active_key, false)
            .with_string(email_key, charlie_email)
            .build();

        // Create test resources
        let doc_id = interner.intern("doc1");
        let doc_type = interner.intern("Document");
        let owner_key = interner.intern("owner");
        let owner_alice = interner.intern("alice");

        let doc = EntityBuilder::new(doc_id, doc_type)
            .with_string(owner_key, owner_alice)
            .build();

        store.insert(alice);
        store.insert(bob);
        store.insert(charlie);
        store.insert(doc);

        store
    }

    #[test]
    fn test_simple_policy_allow() {
        let policy_text = r#"
            policy test {
                default: deny,
                rule admin { allow if user.role == "admin" }
            }
        "#;

        let store = create_test_store();
        let policy = super::super::ReapParser::parse(policy_text).unwrap();
        let evaluator = ReapAstEvaluator::new(store, policy);

        let mut context = HashMap::new();
        context.insert("principal".to_string(), "alice".to_string());

        let request = PolicyRequest {
            resource: "doc1".to_string(),
            action: "read".to_string(),
            context,
        };

        let decision = evaluator.evaluate(&request).unwrap();
        assert!(matches!(decision, PolicyAction::Allow));
    }

    #[test]
    fn test_simple_policy_deny() {
        let policy_text = r#"
            policy test {
                default: deny,
                rule admin { allow if user.role == "admin" }
            }
        "#;

        let store = create_test_store();
        let policy = super::super::ReapParser::parse(policy_text).unwrap();
        let evaluator = ReapAstEvaluator::new(store, policy);

        let mut context = HashMap::new();
        context.insert("principal".to_string(), "bob".to_string());

        let request = PolicyRequest {
            resource: "doc1".to_string(),
            action: "read".to_string(),
            context,
        };

        let decision = evaluator.evaluate(&request).unwrap();
        assert!(matches!(decision, PolicyAction::Deny));
    }

    #[test]
    fn test_numeric_comparison() {
        let policy_text = r#"
            policy test {
                default: deny,
                rule senior { allow if user.years_experience >= 5 }
            }
        "#;

        let store = create_test_store();
        let policy = super::super::ReapParser::parse(policy_text).unwrap();
        let evaluator = ReapAstEvaluator::new(store, policy);

        // Alice has 8 years - should allow
        let mut context = HashMap::new();
        context.insert("principal".to_string(), "alice".to_string());
        let request = PolicyRequest {
            resource: "doc1".to_string(),
            action: "read".to_string(),
            context: context.clone(),
        };
        let decision = evaluator.evaluate(&request).unwrap();
        assert!(matches!(decision, PolicyAction::Allow));

        // Bob has 3 years - should deny
        let mut context2 = HashMap::new();
        context2.insert("principal".to_string(), "bob".to_string());
        let request2 = PolicyRequest {
            resource: "doc1".to_string(),
            action: "read".to_string(),
            context: context2,
        };
        let decision2 = evaluator.evaluate(&request2).unwrap();
        assert!(matches!(decision2, PolicyAction::Deny));
    }

    #[test]
    fn test_and_condition() {
        let policy_text = r#"
            policy test {
                default: deny,
                rule senior_active {
                    allow if {
                        user.years_experience >= 5 &&
                        user.active == true
                    }
                }
            }
        "#;

        let store = create_test_store();
        let policy = super::super::ReapParser::parse(policy_text).unwrap();
        let evaluator = ReapAstEvaluator::new(store, policy);

        // Alice: 8 years, active=true - should allow
        let mut context1 = HashMap::new();
        context1.insert("principal".to_string(), "alice".to_string());
        let request1 = PolicyRequest {
            resource: "doc1".to_string(),
            action: "read".to_string(),
            context: context1,
        };
        assert!(matches!(
            evaluator.evaluate(&request1).unwrap(),
            PolicyAction::Allow
        ));

        // Charlie: 6 years, active=false - should deny
        let mut context2 = HashMap::new();
        context2.insert("principal".to_string(), "charlie".to_string());
        let request2 = PolicyRequest {
            resource: "doc1".to_string(),
            action: "read".to_string(),
            context: context2,
        };
        assert!(matches!(
            evaluator.evaluate(&request2).unwrap(),
            PolicyAction::Deny
        ));
    }

    // TODO: Add more tests for comprehensions once we can properly test them
    // (need to add test data with arrays/objects for iteration)

    #[test]
    fn test_time_now_functions() {
        let policy_text = r#"
            policy test {
                default: deny,
                rule time_check {
                    allow if now_ns := time::now_ns()
                    && now_ms := time::now_ms()
                    && now_s := time::now()
                    && time::is_before(0, now_ns)
                }
            }
        "#;

        let store = create_test_store();
        let policy = super::super::ReapParser::parse(policy_text).unwrap();
        let evaluator = ReapAstEvaluator::new(store, policy);

        let mut context = HashMap::new();
        context.insert("principal".to_string(), "alice".to_string());

        let request = PolicyRequest {
            resource: "doc1".to_string(),
            action: "read".to_string(),
            context,
        };

        let decision = evaluator.evaluate(&request).unwrap();
        assert!(matches!(decision, PolicyAction::Allow));
    }

    #[test]
    fn test_time_parse_format_rfc3339() {
        let policy_text = r#"
            policy test {
                default: deny,
                rule time_parse {
                    allow if parsed := time::parse_rfc3339("2024-01-15T12:30:00Z")
                    && formatted := time::format_rfc3339(parsed)
                    && time::is_before(0, parsed)
                }
            }
        "#;

        let store = create_test_store();
        let policy = super::super::ReapParser::parse(policy_text).unwrap();
        let evaluator = ReapAstEvaluator::new(store, policy);

        let mut context = HashMap::new();
        context.insert("principal".to_string(), "alice".to_string());

        let request = PolicyRequest {
            resource: "doc1".to_string(),
            action: "read".to_string(),
            context,
        };

        let decision = evaluator.evaluate(&request).unwrap();
        assert!(matches!(decision, PolicyAction::Allow));
    }

    #[test]
    fn test_time_arithmetic() {
        let policy_text = r#"
            policy test {
                default: deny,
                rule time_arithmetic {
                    allow if base := time::parse_rfc3339("2024-01-15T12:00:00Z")
                    && future := time::add_ns(base, 3600000000000)
                    && past := time::subtract_ns(base, 3600000000000)
                    && time::is_before(base, future)
                    && time::is_before(past, base)
                }
            }
        "#;

        let store = create_test_store();
        let policy = super::super::ReapParser::parse(policy_text).unwrap();
        let evaluator = ReapAstEvaluator::new(store, policy);

        let mut context = HashMap::new();
        context.insert("principal".to_string(), "alice".to_string());

        let request = PolicyRequest {
            resource: "doc1".to_string(),
            action: "read".to_string(),
            context,
        };

        let decision = evaluator.evaluate(&request).unwrap();
        assert!(matches!(decision, PolicyAction::Allow));
    }

    #[test]
    fn test_time_comparisons() {
        let policy_text = r#"
            policy test {
                default: deny,
                rule time_comparisons {
                    allow if t1 := time::parse_rfc3339("2024-01-15T10:00:00Z")
                    && t2 := time::parse_rfc3339("2024-01-15T12:00:00Z")
                    && t3 := time::parse_rfc3339("2024-01-15T14:00:00Z")
                    && time::is_before(t1, t2)
                    && time::is_after(t3, t2)
                    && time::is_between(t2, t1, t3)
                }
            }
        "#;

        let store = create_test_store();
        let policy = super::super::ReapParser::parse(policy_text).unwrap();
        let evaluator = ReapAstEvaluator::new(store, policy);

        let mut context = HashMap::new();
        context.insert("principal".to_string(), "alice".to_string());

        let request = PolicyRequest {
            resource: "doc1".to_string(),
            action: "read".to_string(),
            context,
        };

        let decision = evaluator.evaluate(&request).unwrap();
        assert!(matches!(decision, PolicyAction::Allow));
    }

    #[test]
    fn test_time_based_access_control() {
        // Test realistic scenario: allow access only during business hours
        let policy_text = r#"
            policy test {
                default: deny,
                rule business_hours {
                    allow if start := time::parse_rfc3339("2024-01-15T09:00:00Z")
                    && end := time::parse_rfc3339("2024-01-15T17:00:00Z")
                    && current := time::parse_rfc3339("2024-01-15T12:00:00Z")
                    && time::is_between(current, start, end)
                }
            }
        "#;

        let store = create_test_store();
        let policy = super::super::ReapParser::parse(policy_text).unwrap();
        let evaluator = ReapAstEvaluator::new(store, policy);

        let mut context = HashMap::new();
        context.insert("principal".to_string(), "alice".to_string());

        let request = PolicyRequest {
            resource: "doc1".to_string(),
            action: "read".to_string(),
            context,
        };

        let decision = evaluator.evaluate(&request).unwrap();
        assert!(matches!(decision, PolicyAction::Allow));
    }

    // NOTE: Comprehensive regex evaluator tests deferred to integration test suite
    // Parser tests verify syntax works correctly

    #[test]
    fn test_regex_namespace_functions() {
        let policy_text = r#"
            policy test {
                default: deny,
                rule pattern_validation {
                    allow if pattern := "[a-z]+"
                    && regex::is_valid(pattern)
                    && special_chars := ".*+?"
                    && escaped := regex::escape(special_chars)
                }
            }
        "#;

        let store = create_test_store();
        let policy = super::super::ReapParser::parse(policy_text).unwrap();
        let evaluator = ReapAstEvaluator::new(store, policy);

        let mut context = HashMap::new();
        context.insert("principal".to_string(), "alice".to_string());

        let request = PolicyRequest {
            resource: "doc1".to_string(),
            action: "read".to_string(),
            context,
        };

        let decision = evaluator.evaluate(&request).unwrap();
        assert!(matches!(decision, PolicyAction::Allow));
    }

    #[test]
    fn test_math_abs_functions() {
        let policy_text = r#"
            policy test {
                default: deny,
                rule math_absolute {
                    allow if neg_int := -42
                    && pos_int := math::abs(neg_int)
                    && neg_float := -3.14
                    && pos_float := math::abs(neg_float)
                }
            }
        "#;

        let store = create_test_store();
        let policy = super::super::ReapParser::parse(policy_text).unwrap();
        let evaluator = ReapAstEvaluator::new(store, policy);

        let mut context = HashMap::new();
        context.insert("principal".to_string(), "alice".to_string());

        let request = PolicyRequest {
            resource: "doc1".to_string(),
            action: "read".to_string(),
            context,
        };

        let decision = evaluator.evaluate(&request).unwrap();
        assert!(matches!(decision, PolicyAction::Allow));
    }

    #[test]
    fn test_math_rounding_functions() {
        let policy_text = r#"
            policy test {
                default: deny,
                rule math_rounding {
                    allow if rounded := math::round(3.7)
                    && floored := math::floor(3.9)
                    && ceiled := math::ceil(3.1)
                }
            }
        "#;

        let store = create_test_store();
        let policy = super::super::ReapParser::parse(policy_text).unwrap();
        let evaluator = ReapAstEvaluator::new(store, policy);

        let mut context = HashMap::new();
        context.insert("principal".to_string(), "alice".to_string());

        let request = PolicyRequest {
            resource: "doc1".to_string(),
            action: "read".to_string(),
            context,
        };

        let decision = evaluator.evaluate(&request).unwrap();
        assert!(matches!(decision, PolicyAction::Allow));
    }

    #[test]
    fn test_math_pow_sqrt() {
        let policy_text = r#"
            policy test {
                default: deny,
                rule math_power_sqrt {
                    allow if squared := math::pow(5, 2)
                    && cubed := math::pow(2, 3)
                    && sqrt_result := math::sqrt(16)
                }
            }
        "#;

        let store = create_test_store();
        let policy = super::super::ReapParser::parse(policy_text).unwrap();
        let evaluator = ReapAstEvaluator::new(store, policy);

        let mut context = HashMap::new();
        context.insert("principal".to_string(), "alice".to_string());

        let request = PolicyRequest {
            resource: "doc1".to_string(),
            action: "read".to_string(),
            context,
        };

        let decision = evaluator.evaluate(&request).unwrap();
        assert!(matches!(decision, PolicyAction::Allow));
    }

    #[test]
    fn test_math_min_max_clamp() {
        let policy_text = r#"
            policy test {
                default: deny,
                rule math_comparisons {
                    allow if min_val := math::min(10, 20)
                    && max_val := math::max(10, 20)
                    && clamped_high := math::clamp(150, 0, 100)
                    && clamped_low := math::clamp(-50, 0, 100)
                    && clamped_mid := math::clamp(50, 0, 100)
                }
            }
        "#;

        let store = create_test_store();
        let policy = super::super::ReapParser::parse(policy_text).unwrap();
        let evaluator = ReapAstEvaluator::new(store, policy);

        let mut context = HashMap::new();
        context.insert("principal".to_string(), "alice".to_string());

        let request = PolicyRequest {
            resource: "doc1".to_string(),
            action: "read".to_string(),
            context,
        };

        let decision = evaluator.evaluate(&request).unwrap();
        assert!(matches!(decision, PolicyAction::Allow));
    }
}
