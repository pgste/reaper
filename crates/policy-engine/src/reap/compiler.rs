//! Compiler: AST → ReaperDSLEvaluator
//!
//! Transforms parsed .reap AST into optimized ReaperDSLEvaluator for sub-microsecond evaluation.

use super::ast::{
    AssignmentValue, ComparisonLeft, ComparisonRight, Condition, Decision, Entity, EntityAttr,
    Expr, MethodName, Operator, Policy, Rule, Value,
};
use crate::evaluators::reaper_dsl::{
    AttrCompareOp, Condition as DslCondition, ReaperDSLEvaluator, Rule as DslRule,
};
use crate::{data::DataStore, PolicyAction};
use reaper_core::ReaperError;
use std::sync::Arc;

/// Compile a parsed policy AST into a ReaperDSLEvaluator
pub fn compile_policy(
    policy: Policy,
    store: Arc<DataStore>,
) -> Result<ReaperDSLEvaluator, ReaperError> {
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

        Condition::Comparison { left, op, right } => compile_comparison(left, op, right),

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

        Condition::Assignment { variable, value } => {
            // Check if this is a comprehension assignment
            if matches!(value, AssignmentValue::Comprehension(_)) {
                return Err(ReaperError::InvalidPolicy {
                    reason: format!(
                        "Comprehensions are not yet supported in compiled policies. \
                        Variable '{}' uses a comprehension which requires direct AST evaluation. \
                        Full comprehension support coming in next release.",
                        variable
                    ),
                });
            }

            // Check if this is an expression assignment (e.g., function call)
            if matches!(value, AssignmentValue::Expr(_)) {
                return Err(ReaperError::InvalidPolicy {
                    reason: format!(
                        "Expression assignments (e.g., function calls) are not yet supported in compiled policies. \
                        Variable '{}' uses an expression which requires direct AST evaluation. \
                        Use .reap format with direct evaluation for expression support.",
                        variable
                    ),
                });
            }

            // Regular assignments not yet supported in compiler
            Err(ReaperError::InvalidPolicy {
                reason: "Variable assignments (:=) are not yet supported in compiled policies. \
                        Use .reap format with direct evaluation for variable support."
                    .to_string(),
            })
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

            Ok(DslCondition::TimeIsAfter {
                entity_type,
                attribute,
                threshold,
            })
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

            Ok(DslCondition::TimeIsBefore {
                entity_type,
                attribute,
                threshold,
            })
        }

        // Type check functions: is_string, is_number, is_bool
        ("", "is_string") => {
            if args.len() != 1 {
                return Err(ReaperError::InvalidPolicy {
                    reason: format!("is_string requires 1 argument, got {}", args.len()),
                });
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

/// Compile a method call expression (e.g., user.email.contains("@"))
fn compile_method_call(
    receiver: Expr,
    method: MethodName,
    args: Vec<Expr>,
) -> Result<DslCondition, ReaperError> {
    // Extract entity type and attribute from receiver
    let (entity_type, attribute) = extract_entity_attr(&receiver)?;

    match method {
        MethodName::Contains => {
            if args.len() != 1 {
                return Err(ReaperError::InvalidPolicy {
                    reason: format!(".contains() requires 1 argument, got {}", args.len()),
                });
            }
            let substring = extract_string_literal(&args[0])?;
            Ok(DslCondition::StringContains {
                entity_type,
                attribute,
                substring,
            })
        }

        MethodName::Startswith => {
            if args.len() != 1 {
                return Err(ReaperError::InvalidPolicy {
                    reason: format!(".startswith() requires 1 argument, got {}", args.len()),
                });
            }
            let prefix = extract_string_literal(&args[0])?;
            Ok(DslCondition::StringStartsWith {
                entity_type,
                attribute,
                prefix,
            })
        }

        MethodName::Endswith => {
            if args.len() != 1 {
                return Err(ReaperError::InvalidPolicy {
                    reason: format!(".endswith() requires 1 argument, got {}", args.len()),
                });
            }
            let suffix = extract_string_literal(&args[0])?;
            Ok(DslCondition::StringEndsWith {
                entity_type,
                attribute,
                suffix,
            })
        }

        MethodName::Matches => {
            // .matches("pattern") is an alias for regex::matches
            if args.len() != 1 {
                return Err(ReaperError::InvalidPolicy {
                    reason: format!(".matches() requires 1 argument, got {}", args.len()),
                });
            }
            let pattern = extract_string_literal(&args[0])?;

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

        _ => Err(ReaperError::InvalidPolicy {
            reason: format!(
                "Method .{}() is not supported in compiled policies. \
                Supported methods: .contains(), .startswith(), .endswith(), .matches()",
                method.as_str()
            ),
        }),
    }
}

/// Extract entity type and attribute from an expression
/// Supports: user.attr, resource.attr, context.attr
/// Also handles Variable("user.email") format from parser
fn extract_entity_attr(
    expr: &Expr,
) -> Result<(crate::evaluators::reaper_dsl::EntityType, String), ReaperError> {
    use crate::evaluators::reaper_dsl::EntityType;

    match expr {
        Expr::AttributeAccess {
            variable,
            attribute,
        } => {
            let entity_type = match variable.as_str() {
                "user" => EntityType::User,
                "resource" => EntityType::Resource,
                "context" => EntityType::Context,
                _ => {
                    return Err(ReaperError::InvalidPolicy {
                        reason: format!(
                            "Unknown entity type '{}'. Expected 'user', 'resource', or 'context'",
                            variable
                        ),
                    })
                }
            };
            Ok((entity_type, attribute.clone()))
        }

        // Handle Variable("user.email") format - split on dot
        Expr::Variable(var_name) => {
            if let Some((entity, attr)) = var_name.split_once('.') {
                let entity_type = match entity {
                    "user" => EntityType::User,
                    "resource" => EntityType::Resource,
                    "context" => EntityType::Context,
                    _ => {
                        return Err(ReaperError::InvalidPolicy {
                            reason: format!(
                            "Unknown entity type '{}'. Expected 'user', 'resource', or 'context'",
                            entity
                        ),
                        })
                    }
                };
                Ok((entity_type, attr.to_string()))
            } else {
                Err(ReaperError::InvalidPolicy {
                    reason: format!(
                        "Variable '{}' is not a valid entity.attribute format",
                        var_name
                    ),
                })
            }
        }

        _ => Err(ReaperError::InvalidPolicy {
            reason: format!(
                "Expected entity.attribute access (e.g., user.email), got {:?}",
                expr
            ),
        }),
    }
}

/// Extract a string literal from an expression
fn extract_string_literal(expr: &Expr) -> Result<String, ReaperError> {
    match expr {
        Expr::Literal(Value::String(s)) => Ok(s.clone()),
        _ => Err(ReaperError::InvalidPolicy {
            reason: format!("Expected string literal, got {:?}", expr),
        }),
    }
}

/// Extract an integer literal from an expression
fn extract_int_literal(expr: &Expr) -> Result<i64, ReaperError> {
    match expr {
        Expr::Literal(Value::Integer(i)) => Ok(*i),
        _ => Err(ReaperError::InvalidPolicy {
            reason: format!("Expected integer literal, got {:?}", expr),
        }),
    }
}

/// Compile expression comparison: user.skills.count() >= 5, user.name.lower() == "admin"
fn compile_expr_comparison(
    expr: Expr,
    op: Operator,
    right: ComparisonRight,
) -> Result<DslCondition, ReaperError> {
    // Handle method calls like user.skills.count(), user.name.lower()
    if let Expr::MethodCall {
        receiver,
        method,
        args: _,
    } = expr
    {
        let (entity_type, attribute) = extract_entity_attr(&receiver)?;

        // Handle .count() method - requires integer on right side
        if method == MethodName::Count {
            let threshold = match right {
                ComparisonRight::Value(Value::Integer(i)) => i as usize,
                ComparisonRight::Value(Value::Float(f)) => f as usize,
                _ => {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "Method call comparisons (e.g., .count()) require integer literal on right side".to_string(),
                    })
                }
            };

            return match op {
                Operator::GreaterEqual => Ok(DslCondition::CountGreaterEqual {
                    entity_type,
                    attribute,
                    threshold,
                }),
                Operator::GreaterThan => Ok(DslCondition::CountGreater {
                    entity_type,
                    attribute,
                    threshold,
                }),
                Operator::Equal => Ok(DslCondition::CountEqual {
                    entity_type,
                    attribute,
                    threshold,
                }),
                Operator::LessEqual => {
                    // count <= N is same as NOT(count > N)
                    Ok(DslCondition::Not(Box::new(DslCondition::CountGreater {
                        entity_type,
                        attribute,
                        threshold,
                    })))
                }
                Operator::LessThan => {
                    // count < N is same as NOT(count >= N)
                    Ok(DslCondition::Not(Box::new(
                        DslCondition::CountGreaterEqual {
                            entity_type,
                            attribute,
                            threshold,
                        },
                    )))
                }
                _ => Err(ReaperError::InvalidPolicy {
                    reason: format!("Operator {:?} not supported for .count() comparisons", op),
                }),
            };
        }

        // Handle .lower() method - user.name.lower() == "admin"
        if method == MethodName::Lower {
            let value = match right {
                ComparisonRight::Value(Value::String(s)) => s,
                _ => {
                    return Err(ReaperError::InvalidPolicy {
                        reason: ".lower() comparisons require string literal on right side"
                            .to_string(),
                    })
                }
            };

            return match op {
                Operator::Equal => Ok(DslCondition::StringLowerEquals {
                    entity_type,
                    attribute,
                    value,
                }),
                Operator::NotEqual => Ok(DslCondition::Not(Box::new(
                    DslCondition::StringLowerEquals {
                        entity_type,
                        attribute,
                        value,
                    },
                ))),
                _ => Err(ReaperError::InvalidPolicy {
                    reason: format!(
                        "Operator {:?} not supported for .lower() comparisons. Use == or !=",
                        op
                    ),
                }),
            };
        }

        // Handle .upper() method - user.code.upper() == "ADMIN"
        if method == MethodName::Upper {
            let value = match right {
                ComparisonRight::Value(Value::String(s)) => s,
                _ => {
                    return Err(ReaperError::InvalidPolicy {
                        reason: ".upper() comparisons require string literal on right side"
                            .to_string(),
                    })
                }
            };

            return match op {
                Operator::Equal => Ok(DslCondition::StringUpperEquals {
                    entity_type,
                    attribute,
                    value,
                }),
                Operator::NotEqual => Ok(DslCondition::Not(Box::new(
                    DslCondition::StringUpperEquals {
                        entity_type,
                        attribute,
                        value,
                    },
                ))),
                _ => Err(ReaperError::InvalidPolicy {
                    reason: format!(
                        "Operator {:?} not supported for .upper() comparisons. Use == or !=",
                        op
                    ),
                }),
            };
        }

        return Err(ReaperError::InvalidPolicy {
            reason: format!(
                "Method .{}() is not supported in compiled policy comparisons. \
                Supported methods: .count(), .lower(), .upper()",
                method.as_str()
            ),
        });
    }

    Err(ReaperError::InvalidPolicy {
        reason:
            "Expression comparisons only support method calls like .count(), .lower(), .upper()"
                .to_string(),
    })
}

/// Compile a membership test: "admin" in user.roles
fn compile_membership_test(
    left: ComparisonLeft,
    right: ComparisonRight,
) -> Result<DslCondition, ReaperError> {
    // Parser represents "value in collection" as: left=collection, op=In, right=value
    // So left is the entity attribute (collection) and right is the literal value to search for

    // Extract the entity attribute (collection) from the left side
    let (entity_type, attribute) = match left {
        ComparisonLeft::EntityAttr(attr) => {
            let entity_type = match attr.entity {
                Entity::User => crate::evaluators::reaper_dsl::EntityType::User,
                Entity::Resource => crate::evaluators::reaper_dsl::EntityType::Resource,
                Entity::Context => crate::evaluators::reaper_dsl::EntityType::Context,
            };
            (entity_type, attr.attribute)
        }
        _ => {
            return Err(ReaperError::InvalidPolicy {
                reason: "Left side of 'in' operator should be an entity attribute collection (e.g., user.roles)".to_string(),
            })
        }
    };

    // Extract the literal value from the right side
    let literal_value = match right {
        ComparisonRight::Value(value) => match value {
            Value::String(s) => crate::evaluators::reaper_dsl::LiteralValue::String(s),
            Value::Integer(i) => crate::evaluators::reaper_dsl::LiteralValue::Int(i),
            Value::Boolean(b) => crate::evaluators::reaper_dsl::LiteralValue::Bool(b),
            _ => {
                return Err(ReaperError::InvalidPolicy {
                    reason: "Only string, integer, and boolean literals are supported in membership tests".to_string(),
                })
            }
        },
        _ => {
            return Err(ReaperError::InvalidPolicy {
                reason: "Right side of 'in' operator should be a literal value (e.g., \"admin\" in user.roles)".to_string(),
            })
        }
    };

    Ok(DslCondition::MembershipTest {
        value: literal_value,
        entity_type,
        attribute,
        index: None, // TODO: Support indexed membership tests like "admin" in user.groups[0].members
    })
}

/// Compile a comparison into the appropriate DslCondition variant
fn compile_comparison(
    left: ComparisonLeft,
    op: Operator,
    right: ComparisonRight,
) -> Result<DslCondition, ReaperError> {
    // Special case: check if this is an "action" or "resource" variable comparison
    if let ComparisonLeft::Expr(Expr::Variable(var_name)) = &left {
        if var_name == "action" || var_name == "resource" {
            // Handle action == "value" and resource == "value" comparisons
            if let ComparisonRight::Value(value) = right {
                let value_str = match value {
                    Value::String(s) => s,
                    Value::Integer(i) => i.to_string(),
                    Value::Float(f) => f.to_string(),
                    Value::Boolean(b) => b.to_string(),
                    Value::Null => "null".to_string(),
                    _ => {
                        return Err(ReaperError::InvalidPolicy {
                            reason: format!(
                                "{} comparisons only support simple literal values",
                                var_name
                            ),
                        })
                    }
                };
                return match (var_name.as_str(), op) {
                    ("action", Operator::Equal) => {
                        Ok(DslCondition::ActionEquals { value: value_str })
                    }
                    ("action", Operator::NotEqual) => {
                        Ok(DslCondition::Not(Box::new(DslCondition::ActionEquals {
                            value: value_str,
                        })))
                    }
                    ("resource", Operator::Equal) => {
                        Ok(DslCondition::ResourceIdEquals { value: value_str })
                    }
                    ("resource", Operator::NotEqual) => Ok(DslCondition::Not(Box::new(
                        DslCondition::ResourceIdEquals { value: value_str },
                    ))),
                    _ => Err(ReaperError::InvalidPolicy {
                        reason: format!(
                            "Operator {:?} not supported for {} comparisons. Use == or !=.",
                            op, var_name
                        ),
                    }),
                };
            } else {
                return Err(ReaperError::InvalidPolicy {
                    reason: format!(
                        "{} comparisons must be against literal values (e.g., {} == \"value\")",
                        var_name, var_name
                    ),
                });
            }
        }
    }

    // Handle "in" operator for membership tests: "admin" in user.roles
    if op == Operator::In {
        return compile_membership_test(left, right);
    }

    // Extract EntityAttr from left - var attributes not supported in compiler
    let left_attr = match left {
        ComparisonLeft::EntityAttr(attr) => attr,
        ComparisonLeft::VarAttr(var_attr) => {
            return Err(ReaperError::InvalidPolicy {
                reason: format!(
                    "Variable attribute access '{}.{}' is not supported in compiled policies. \
                    Variable attributes require direct AST evaluation. \
                    Use .reap format with direct evaluation for comprehension filter support.",
                    var_attr.variable, var_attr.attribute
                ),
            });
        }
        ComparisonLeft::Expr(expr) => {
            // Handle method calls like user.skills.count() >= 5
            return compile_expr_comparison(expr, op, right);
        }
    };

    match right {
        ComparisonRight::Value(value) => compile_value_comparison(left_attr, op, value),
        ComparisonRight::EntityAttr(right_attr) => {
            compile_attr_comparison(left_attr, op, right_attr)
        }
        ComparisonRight::VarAttr(var_attr) => Err(ReaperError::InvalidPolicy {
            reason: format!(
                "Variable attribute access '{}.{}' is not supported in compiled policies. \
                    Variable attributes require direct AST evaluation.",
                var_attr.variable, var_attr.attribute
            ),
        }),
        ComparisonRight::Variable(_) => Err(ReaperError::InvalidPolicy {
            reason: "Variable references are not yet supported in compiled policies".to_string(),
        }),
        ComparisonRight::Expr(_) => Err(ReaperError::InvalidPolicy {
            reason: "Expression comparisons (method calls, etc.) are not supported in compiled policies. \
                Use .reap format with AST evaluation for expression support.".to_string(),
        }),
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
        Value::Array(arr) => {
            // Serialize array to JSON string for comparison
            serde_json::to_string(&arr).map_err(|e| ReaperError::InvalidPolicy {
                reason: format!("Failed to serialize array: {}", e),
            })?
        }
        Value::Object(obj) => {
            // Serialize object to JSON string for comparison
            serde_json::to_string(&obj).map_err(|e| ReaperError::InvalidPolicy {
                reason: format!("Failed to serialize object: {}", e),
            })?
        }
        Value::Set(set) => {
            // Serialize set to JSON string for comparison
            serde_json::to_string(&set).map_err(|e| ReaperError::InvalidPolicy {
                reason: format!("Failed to serialize set: {}", e),
            })?
        }
    };

    match (left.entity, op) {
        // User attribute comparisons
        (Entity::User, Operator::Equal) => Ok(DslCondition::UserEquals {
            attribute: left.attribute,
            value: value_str,
        }),

        (Entity::User, Operator::NotEqual) => {
            Ok(DslCondition::Not(Box::new(DslCondition::UserEquals {
                attribute: left.attribute,
                value: value_str,
            })))
        }

        // User numeric comparisons (>=, >, <=, <)
        (Entity::User, Operator::GreaterEqual) => {
            let num_value = value_str
                .parse::<f64>()
                .map_err(|_| ReaperError::InvalidPolicy {
                    reason: format!(
                        "Cannot compare attribute to non-numeric value '{}' with >=",
                        value_str
                    ),
                })?;
            Ok(DslCondition::UserGreaterEqualLiteral {
                attribute: left.attribute,
                value: num_value,
            })
        }

        (Entity::User, Operator::GreaterThan) => {
            let num_value = value_str
                .parse::<f64>()
                .map_err(|_| ReaperError::InvalidPolicy {
                    reason: format!(
                        "Cannot compare attribute to non-numeric value '{}' with >",
                        value_str
                    ),
                })?;
            Ok(DslCondition::UserGreaterLiteral {
                attribute: left.attribute,
                value: num_value,
            })
        }

        (Entity::User, Operator::LessEqual) => {
            let num_value = value_str
                .parse::<f64>()
                .map_err(|_| ReaperError::InvalidPolicy {
                    reason: format!(
                        "Cannot compare attribute to non-numeric value '{}' with <=",
                        value_str
                    ),
                })?;
            Ok(DslCondition::UserLessEqualLiteral {
                attribute: left.attribute,
                value: num_value,
            })
        }

        (Entity::User, Operator::LessThan) => {
            let num_value = value_str
                .parse::<f64>()
                .map_err(|_| ReaperError::InvalidPolicy {
                    reason: format!(
                        "Cannot compare attribute to non-numeric value '{}' with <",
                        value_str
                    ),
                })?;
            Ok(DslCondition::UserLessLiteral {
                attribute: left.attribute,
                value: num_value,
            })
        }

        // Resource attribute comparisons
        (Entity::Resource, Operator::Equal) => Ok(DslCondition::ResourceEquals {
            attribute: left.attribute,
            value: value_str,
        }),

        (Entity::Resource, Operator::NotEqual) => {
            Ok(DslCondition::Not(Box::new(DslCondition::ResourceEquals {
                attribute: left.attribute,
                value: value_str,
            })))
        }

        (Entity::Resource, Operator::GreaterEqual) => {
            let num_value = value_str
                .parse::<f64>()
                .map_err(|_| ReaperError::InvalidPolicy {
                    reason: format!(
                        "Cannot compare attribute to non-numeric value '{}' with >=",
                        value_str
                    ),
                })?;
            Ok(DslCondition::ResourceGreaterEqualLiteral {
                attribute: left.attribute,
                value: num_value,
            })
        }

        (Entity::Resource, Operator::GreaterThan) => {
            let num_value = value_str
                .parse::<f64>()
                .map_err(|_| ReaperError::InvalidPolicy {
                    reason: format!(
                        "Cannot compare attribute to non-numeric value '{}' with >",
                        value_str
                    ),
                })?;
            Ok(DslCondition::ResourceGreaterLiteral {
                attribute: left.attribute,
                value: num_value,
            })
        }

        (Entity::Resource, Operator::LessEqual) => {
            let num_value = value_str
                .parse::<f64>()
                .map_err(|_| ReaperError::InvalidPolicy {
                    reason: format!(
                        "Cannot compare attribute to non-numeric value '{}' with <=",
                        value_str
                    ),
                })?;
            Ok(DslCondition::ResourceLessEqualLiteral {
                attribute: left.attribute,
                value: num_value,
            })
        }

        (Entity::Resource, Operator::LessThan) => {
            let num_value = value_str
                .parse::<f64>()
                .map_err(|_| ReaperError::InvalidPolicy {
                    reason: format!(
                        "Cannot compare attribute to non-numeric value '{}' with <",
                        value_str
                    ),
                })?;
            Ok(DslCondition::ResourceLessLiteral {
                attribute: left.attribute,
                value: num_value,
            })
        }

        // Context not yet supported
        (Entity::Context, _) => Err(ReaperError::InvalidPolicy {
            reason: "Context entity not yet supported".to_string(),
        }),

        // Unsupported operators for value comparisons
        _ => Err(ReaperError::InvalidPolicy {
            reason: format!(
                "Operator {:?} not supported for literal value comparisons. Use == or != instead.",
                op
            ),
        }),
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
        (Entity::User, Entity::Resource, Operator::Equal) => Ok(DslCondition::UserEqualsResource {
            user_attr: left.attribute,
            resource_attr: right.attribute,
        }),

        // User > Resource (int comparison)
        (Entity::User, Entity::Resource, Operator::GreaterThan) => {
            Ok(DslCondition::UserIntGreater {
                user_attr: left.attribute,
                resource_attr: right.attribute,
            })
        }

        // User >= Resource (user > resource || user == resource)
        (Entity::User, Entity::Resource, Operator::GreaterEqual) => Ok(DslCondition::Or(vec![
            DslCondition::UserIntGreater {
                user_attr: left.attribute.clone(),
                resource_attr: right.attribute.clone(),
            },
            DslCondition::UserEqualsResource {
                user_attr: left.attribute,
                resource_attr: right.attribute,
            },
        ])),

        // Resource > User
        (Entity::Resource, Entity::User, Operator::GreaterThan) => {
            Ok(DslCondition::ResourceIntGreater {
                resource_attr: left.attribute,
                user_attr: right.attribute,
            })
        }

        // Resource >= User
        (Entity::Resource, Entity::User, Operator::GreaterEqual) => Ok(DslCondition::Or(vec![
            DslCondition::ResourceIntGreater {
                resource_attr: left.attribute.clone(),
                user_attr: right.attribute.clone(),
            },
            DslCondition::UserEqualsResource {
                user_attr: right.attribute,
                resource_attr: left.attribute,
            },
        ])),

        // User < Resource = Resource > User
        (Entity::User, Entity::Resource, Operator::LessThan) => {
            Ok(DslCondition::ResourceIntGreater {
                resource_attr: right.attribute,
                user_attr: left.attribute,
            })
        }

        // User <= Resource = Resource >= User
        (Entity::User, Entity::Resource, Operator::LessEqual) => Ok(DslCondition::Or(vec![
            DslCondition::ResourceIntGreater {
                resource_attr: right.attribute.clone(),
                user_attr: left.attribute.clone(),
            },
            DslCondition::UserEqualsResource {
                user_attr: left.attribute,
                resource_attr: right.attribute,
            },
        ])),

        // User != Resource
        (Entity::User, Entity::Resource, Operator::NotEqual) => Ok(DslCondition::Not(Box::new(
            DslCondition::UserEqualsResource {
                user_attr: left.attribute,
                resource_attr: right.attribute,
            },
        ))),

        // Same-entity comparisons: entity.attr1 op entity.attr2
        // Works for User, Resource, or Context entities
        (left_ent, right_ent, op) if left_ent == right_ent => {
            let entity_type = match left_ent {
                Entity::User => crate::evaluators::reaper_dsl::EntityType::User,
                Entity::Resource => crate::evaluators::reaper_dsl::EntityType::Resource,
                Entity::Context => crate::evaluators::reaper_dsl::EntityType::Context,
            };

            let attr_op = match op {
                Operator::LessEqual => AttrCompareOp::LessEqual,
                Operator::GreaterEqual => AttrCompareOp::GreaterEqual,
                Operator::LessThan => AttrCompareOp::Less,
                Operator::GreaterThan => AttrCompareOp::Greater,
                Operator::Equal => AttrCompareOp::Equal,
                Operator::NotEqual => AttrCompareOp::NotEqual,
                _ => {
                    return Err(ReaperError::InvalidPolicy {
                        reason: format!(
                            "Operator {:?} not supported for same-entity comparisons",
                            op
                        ),
                    })
                }
            };

            Ok(DslCondition::SameEntityAttrCompare {
                entity_type,
                left_attr: left.attribute,
                right_attr: right.attribute,
                op: attr_op,
            })
        }

        // Unsupported combinations
        _ => Err(ReaperError::InvalidPolicy {
            reason: format!(
                "Unsupported comparison: {:?}.{} {:?} {:?}.{}",
                left.entity, left.attribute, op, right.entity, right.attribute
            ),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evaluators::PolicyEvaluator;
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
        };

        let decision = evaluator.evaluate(&request).unwrap();
        assert!(matches!(decision, PolicyAction::Allow));
    }
}
