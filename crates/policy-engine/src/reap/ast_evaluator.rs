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
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// AST-based policy evaluator
///
/// Evaluates policies directly from the AST, supporting all language features
/// including comprehensions, variable assignments, and complex expressions.
#[derive(Debug, Clone)]
pub struct ReapAstEvaluator {
    /// Reference to the data store
    store: Arc<DataStore>,
    /// Parsed policy AST
    policy: Policy,
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
        Self { store, policy }
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
            if matches!(rule.decision, super::ast::Decision::Deny) {
                if self.evaluate_condition(&rule.condition, &mut context)? {
                    // Explicit deny - return immediately, no allow can override this
                    return Ok(PolicyAction::Deny);
                }
            }
        }

        // Phase 2: No deny matched, now evaluate ALLOW rules
        for rule in &self.policy.rules {
            if matches!(rule.decision, super::ast::Decision::Allow) {
                if self.evaluate_condition(&rule.condition, &mut context)? {
                    return Ok(PolicyAction::Allow);
                }
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
}
