//! Compiler: AST → ReaperDSLEvaluator
//!
//! Transforms parsed .reap AST into optimized ReaperDSLEvaluator for sub-microsecond evaluation.

use super::ast::*;
use crate::evaluators::reaper_dsl::{ReaperDSLEvaluator, Rule as DslRule, Condition as DslCondition};
use crate::{PolicyAction, data::DataStore};
use reaper_core::ReaperError;
use std::sync::Arc;

/// Compile a parsed policy AST into a ReaperDSLEvaluator
pub fn compile_policy(policy: Policy, store: Arc<DataStore>) -> Result<ReaperDSLEvaluator, ReaperError> {
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

    Ok(ReaperDSLEvaluator::new(store, rules, default_decision))
}

/// Compile a single rule
fn compile_rule(rule: Rule) -> Result<DslRule, ReaperError> {
    let decision = match rule.decision {
        Decision::Allow => PolicyAction::Allow,
        Decision::Deny => PolicyAction::Deny,
    };

    let condition = compile_condition(rule.condition)?;

    Ok(DslRule {
        name: rule.name,
        condition,
        decision,
    })
}

/// Compile a condition expression
fn compile_condition(cond: Condition) -> Result<DslCondition, ReaperError> {
    match cond {
        Condition::True => Ok(DslCondition::Always),

        Condition::False => {
            // False condition = Not(Always)
            Ok(DslCondition::Not(Box::new(DslCondition::Always)))
        }

        Condition::Comparison { left, op, right } => {
            compile_comparison(left, op, right)
        }

        Condition::And(conditions) => {
            let mut compiled = Vec::new();
            for c in conditions {
                compiled.push(compile_condition(c)?);
            }
            Ok(DslCondition::And(compiled))
        }

        Condition::Or(conditions) => {
            let mut compiled = Vec::new();
            for c in conditions {
                compiled.push(compile_condition(c)?);
            }
            Ok(DslCondition::Or(compiled))
        }

        Condition::Not(cond) => {
            let compiled = compile_condition(*cond)?;
            Ok(DslCondition::Not(Box::new(compiled)))
        }
    }
}

/// Compile a comparison into the appropriate DslCondition variant
fn compile_comparison(
    left: EntityAttr,
    op: Operator,
    right: ComparisonRight,
) -> Result<DslCondition, ReaperError> {
    match right {
        ComparisonRight::Value(value) => {
            compile_value_comparison(left, op, value)
        }
        ComparisonRight::EntityAttr(right_attr) => {
            compile_attr_comparison(left, op, right_attr)
        }
    }
}

/// Compile comparison: entity.attr op literal_value
fn compile_value_comparison(
    left: EntityAttr,
    op: Operator,
    value: Value,
) -> Result<DslCondition, ReaperError> {
    // Get string value for comparison
    let value_str = match value {
        Value::String(s) => s,
        Value::Integer(i) => i.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Boolean(b) => b.to_string(),
        Value::Null => "null".to_string(),
    };

    match (left.entity, op) {
        // User attribute comparisons
        (Entity::User, Operator::Equal) => {
            Ok(DslCondition::UserEquals {
                attribute: left.attribute,
                value: value_str,
            })
        }

        (Entity::User, Operator::NotEqual) => {
            Ok(DslCondition::Not(Box::new(DslCondition::UserEquals {
                attribute: left.attribute,
                value: value_str,
            })))
        }

        // Resource attribute comparisons
        (Entity::Resource, Operator::Equal) => {
            Ok(DslCondition::ResourceEquals {
                attribute: left.attribute,
                value: value_str,
            })
        }

        (Entity::Resource, Operator::NotEqual) => {
            Ok(DslCondition::Not(Box::new(DslCondition::ResourceEquals {
                attribute: left.attribute,
                value: value_str,
            })))
        }

        // Context not yet supported
        (Entity::Context, _) => {
            Err(ReaperError::InvalidPolicy {
                reason: "Context entity not yet supported".to_string(),
            })
        }

        // Unsupported operators for value comparisons
        _ => {
            Err(ReaperError::InvalidPolicy {
                reason: format!(
                    "Operator {:?} not supported for literal value comparisons. Use == or != instead.",
                    op
                ),
            })
        }
    }
}

/// Compile comparison: entity1.attr op entity2.attr
fn compile_attr_comparison(
    left: EntityAttr,
    op: Operator,
    right: EntityAttr,
) -> Result<DslCondition, ReaperError> {
    match (left.entity, right.entity, op) {
        // User == Resource (same attribute)
        (Entity::User, Entity::Resource, Operator::Equal) => {
            Ok(DslCondition::UserEqualsResource {
                user_attr: left.attribute,
                resource_attr: right.attribute,
            })
        }

        // User > Resource (int comparison)
        (Entity::User, Entity::Resource, Operator::GreaterThan) => {
            Ok(DslCondition::UserIntGreater {
                user_attr: left.attribute,
                resource_attr: right.attribute,
            })
        }

        // User >= Resource (user > resource || user == resource)
        (Entity::User, Entity::Resource, Operator::GreaterEqual) => {
            Ok(DslCondition::Or(vec![
                DslCondition::UserIntGreater {
                    user_attr: left.attribute.clone(),
                    resource_attr: right.attribute.clone(),
                },
                DslCondition::UserEqualsResource {
                    user_attr: left.attribute,
                    resource_attr: right.attribute,
                },
            ]))
        }

        // Resource > User
        (Entity::Resource, Entity::User, Operator::GreaterThan) => {
            Ok(DslCondition::ResourceIntGreater {
                resource_attr: left.attribute,
                user_attr: right.attribute,
            })
        }

        // Resource >= User
        (Entity::Resource, Entity::User, Operator::GreaterEqual) => {
            Ok(DslCondition::Or(vec![
                DslCondition::ResourceIntGreater {
                    resource_attr: left.attribute.clone(),
                    user_attr: right.attribute.clone(),
                },
                DslCondition::UserEqualsResource {
                    user_attr: right.attribute,
                    resource_attr: left.attribute,
                },
            ]))
        }

        // User < Resource = Resource > User
        (Entity::User, Entity::Resource, Operator::LessThan) => {
            Ok(DslCondition::ResourceIntGreater {
                resource_attr: right.attribute,
                user_attr: left.attribute,
            })
        }

        // User <= Resource = Resource >= User
        (Entity::User, Entity::Resource, Operator::LessEqual) => {
            Ok(DslCondition::Or(vec![
                DslCondition::ResourceIntGreater {
                    resource_attr: right.attribute.clone(),
                    user_attr: left.attribute.clone(),
                },
                DslCondition::UserEqualsResource {
                    user_attr: left.attribute,
                    resource_attr: right.attribute,
                },
            ]))
        }

        // User != Resource
        (Entity::User, Entity::Resource, Operator::NotEqual) => {
            Ok(DslCondition::Not(Box::new(DslCondition::UserEqualsResource {
                user_attr: left.attribute,
                resource_attr: right.attribute,
            })))
        }

        // Unsupported combinations
        _ => {
            Err(ReaperError::InvalidPolicy {
                reason: format!(
                    "Unsupported comparison: {:?}.{} {:?} {:?}.{}",
                    left.entity, left.attribute, op, right.entity, right.attribute
                ),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{DataStore, EntityBuilder};
    use crate::PolicyRequest;
    use crate::evaluators::PolicyEvaluator;
    use std::collections::HashMap;

    #[test]
    fn test_compile_simple_rule() {
        let policy = Policy {
            name: "test".to_string(),
            metadata: HashMap::new(),
            default_decision: Decision::Deny,
            rules: vec![Rule {
                name: "admin".to_string(),
                decision: Decision::Allow,
                condition: Condition::Comparison {
                    left: EntityAttr {
                        entity: Entity::User,
                        attribute: "role".to_string(),
                    },
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
        };

        let decision = evaluator.evaluate(&request).unwrap();
        assert!(matches!(decision, PolicyAction::Allow));
    }
}
