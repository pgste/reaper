// ! Parser for .reap files using Pest

use super::ast::*;
use pest::Parser as PestParser;
use pest_derive::Parser;
use reaper_core::ReaperError;
use std::collections::HashMap;

#[derive(Parser)]
#[grammar = "reap.pest"]
pub struct ReapParser;

impl ReapParser {
    /// Parse a .reap policy from a string
    pub fn parse(input: &str) -> Result<Policy, ReaperError> {
        let pairs = <Self as PestParser<Rule>>::parse(Rule::policy, input).map_err(|e| {
            ReaperError::InvalidPolicy {
                reason: format!("Parse error: {}", e),
            }
        })?;

        let mut policy_name = String::new();
        let mut metadata = HashMap::new();
        let mut default_decision = None;
        let mut rules = Vec::new();

        for pair in pairs {
            if pair.as_rule() == Rule::policy {
                for inner_pair in pair.into_inner() {
                    match inner_pair.as_rule() {
                        Rule::ident => {
                            policy_name = inner_pair.as_str().to_string();
                        }
                        Rule::policy_body => {
                            for item in inner_pair.into_inner() {
                                match item.as_rule() {
                                    Rule::metadata_field => {
                                        let (key, value) = parse_metadata_field(item)?;
                                        metadata.insert(key, value);
                                    }
                                    Rule::default_field => {
                                        default_decision = Some(parse_default_field(item)?);
                                    }
                                    Rule::rule => {
                                        rules.push(parse_rule(item)?);
                                    }
                                    _ => {}
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        let default_decision = default_decision.ok_or_else(|| ReaperError::InvalidPolicy {
            reason: "Missing 'default' field".to_string(),
        })?;

        Ok(Policy {
            name: policy_name,
            metadata,
            default_decision,
            rules,
        })
    }
}

fn parse_metadata_field(
    pair: pest::iterators::Pair<Rule>,
) -> Result<(String, String), ReaperError> {
    let mut inner = pair.into_inner();
    let key = inner.next().unwrap().as_str().to_string();
    let value_pair = inner.next().unwrap();
    let value = parse_string_literal(value_pair)?;
    Ok((key, value))
}

fn parse_default_field(pair: pest::iterators::Pair<Rule>) -> Result<Decision, ReaperError> {
    let mut inner = pair.into_inner();
    let decision_pair = inner.next().unwrap();
    Ok(Decision::from(decision_pair.as_str()))
}

fn parse_rule(pair: pest::iterators::Pair<Rule>) -> Result<super::ast::Rule, ReaperError> {
    let mut inner = pair.into_inner();

    let name = inner.next().unwrap().as_str().to_string();
    let decision_pair = inner.next().unwrap();
    let decision = Decision::from(decision_pair.as_str());
    let condition_pair = inner.next().unwrap();
    let condition = parse_condition(condition_pair)?;

    Ok(super::ast::Rule {
        name,
        decision,
        condition,
    })
}

fn parse_condition(pair: pest::iterators::Pair<Rule>) -> Result<Condition, ReaperError> {
    let inner = pair.into_inner().next().unwrap();
    match inner.as_rule() {
        Rule::condition_block => {
            let expr = inner.into_inner().next().unwrap();
            parse_condition_expr(expr)
        }
        Rule::condition_expr => parse_condition_expr(inner),
        _ => Err(ReaperError::InvalidPolicy {
            reason: format!("Unexpected rule in condition: {:?}", inner.as_rule()),
        }),
    }
}

fn parse_condition_expr(pair: pest::iterators::Pair<Rule>) -> Result<Condition, ReaperError> {
    let inner = pair.into_inner().next().unwrap();
    parse_or_expr(inner)
}

fn parse_or_expr(pair: pest::iterators::Pair<Rule>) -> Result<Condition, ReaperError> {
    let mut inner = pair.into_inner();
    let first = parse_and_expr(inner.next().unwrap())?;

    let mut conditions = vec![first];
    for and_expr in inner {
        conditions.push(parse_and_expr(and_expr)?);
    }

    if conditions.len() == 1 {
        Ok(conditions.into_iter().next().unwrap())
    } else {
        Ok(Condition::Or(conditions))
    }
}

fn parse_and_expr(pair: pest::iterators::Pair<Rule>) -> Result<Condition, ReaperError> {
    let mut inner = pair.into_inner();
    let first = parse_not_expr(inner.next().unwrap())?;

    let mut conditions = vec![first];
    for not_expr in inner {
        conditions.push(parse_not_expr(not_expr)?);
    }

    if conditions.len() == 1 {
        Ok(conditions.into_iter().next().unwrap())
    } else {
        Ok(Condition::And(conditions))
    }
}

fn parse_not_expr(pair: pest::iterators::Pair<Rule>) -> Result<Condition, ReaperError> {
    let mut inner = pair.into_inner();
    let first = inner.next().unwrap();

    match first.as_rule() {
        Rule::not_expr => {
            let inner_cond = parse_not_expr(first)?;
            Ok(Condition::Not(Box::new(inner_cond)))
        }
        Rule::primary_expr => parse_primary_expr(first),
        _ => Err(ReaperError::InvalidPolicy {
            reason: format!("Unexpected rule in not_expr: {:?}", first.as_rule()),
        }),
    }
}

fn parse_primary_expr(pair: pest::iterators::Pair<Rule>) -> Result<Condition, ReaperError> {
    let inner = pair.into_inner().next().unwrap();

    match inner.as_rule() {
        Rule::condition_expr => parse_condition_expr(inner),
        Rule::assignment => parse_assignment(inner),
        Rule::comparison => parse_comparison(inner),
        Rule::comp_function_call => {
            // Parse function call and wrap it as an expression condition
            let expr = parse_comp_function_call(inner)?;
            Ok(Condition::Expr(expr))
        }
        Rule::boolean_literal => match inner.as_str() {
            "true" => Ok(Condition::True),
            "false" => Ok(Condition::False),
            _ => Err(ReaperError::InvalidPolicy {
                reason: format!("Invalid boolean literal: {}", inner.as_str()),
            }),
        },
        _ => Err(ReaperError::InvalidPolicy {
            reason: format!("Unexpected rule in primary_expr: {:?}", inner.as_rule()),
        }),
    }
}

fn parse_assignment(pair: pest::iterators::Pair<Rule>) -> Result<Condition, ReaperError> {
    let mut inner = pair.into_inner();

    let variable = inner.next().unwrap().as_str().to_string();
    let value_pair = inner.next().unwrap();

    let value = parse_assignment_value(value_pair)?;

    Ok(Condition::Assignment { variable, value })
}

fn parse_assignment_value(
    pair: pest::iterators::Pair<Rule>,
) -> Result<AssignmentValue, ReaperError> {
    // The pair is assignment_value, which contains comprehension, comp_function_call, entity_attr, value, or ident
    let inner = pair
        .into_inner()
        .next()
        .ok_or_else(|| ReaperError::InvalidPolicy {
            reason: "Empty assignment value".to_string(),
        })?;

    match inner.as_rule() {
        Rule::comprehension => Ok(AssignmentValue::Comprehension(parse_comprehension(inner)?)),
        Rule::comp_function_call => Ok(AssignmentValue::Expr(parse_comp_function_call(inner)?)),
        Rule::entity_attr => Ok(AssignmentValue::EntityAttr(parse_entity_attr(inner)?)),
        Rule::value => Ok(AssignmentValue::Value(parse_value(inner)?)),
        Rule::ident => Ok(AssignmentValue::Variable(inner.as_str().to_string())),
        _ => Err(ReaperError::InvalidPolicy {
            reason: format!("Unexpected assignment value type: {:?}", inner.as_rule()),
        }),
    }
}

fn parse_comparison(pair: pest::iterators::Pair<Rule>) -> Result<Condition, ReaperError> {
    let mut inner = pair.into_inner();

    let first_pair = inner.next().unwrap();

    // Check if this is "value in entity_attr" or "value in var_attr" form
    if first_pair.as_rule() == Rule::value {
        // Format: value in attr
        let value = parse_value(first_pair)?;
        // The "in" keyword is implicit in the grammar, not captured
        let next_pair = inner.next().unwrap();

        let left = match next_pair.as_rule() {
            Rule::entity_attr => ComparisonLeft::EntityAttr(parse_entity_attr(next_pair)?),
            Rule::var_attr => ComparisonLeft::VarAttr(parse_var_attr(next_pair)?),
            _ => {
                return Err(ReaperError::InvalidPolicy {
                    reason: format!(
                        "Unexpected attribute type after 'in': {:?}",
                        next_pair.as_rule()
                    ),
                })
            }
        };

        // "value in attr" is represented as: left=attr, op=In, right=value
        return Ok(Condition::Comparison {
            left,
            op: Operator::In,
            right: ComparisonRight::Value(value),
        });
    }

    // Check if this is "variable op something", "entity_attr op something", or "var_attr op something"
    let op_pair = inner.next().unwrap();
    let op = Operator::from(op_pair.as_str());
    let right_pair = inner.next().unwrap();

    // Parse the left side (entity_attr, var_attr, or simple ident)
    let left = match first_pair.as_rule() {
        Rule::entity_attr => ComparisonLeft::EntityAttr(parse_entity_attr(first_pair)?),
        Rule::var_attr => ComparisonLeft::VarAttr(parse_var_attr(first_pair)?),
        Rule::ident => {
            // Simple variable on left side - not supported in comparisons (only in assignments)
            return Err(ReaperError::InvalidPolicy {
                reason: format!(
                    "Variable '{}' cannot appear on left side of comparison (use entity.attribute or var.attribute instead)",
                    first_pair.as_str()
                ),
            });
        }
        _ => {
            return Err(ReaperError::InvalidPolicy {
                reason: format!("Unexpected left side: {:?}", first_pair.as_rule()),
            });
        }
    };

    // Parse the right side using parse_comparison_right
    let right = parse_comparison_right(right_pair)?;

    Ok(Condition::Comparison { left, op, right })
}

fn parse_comparison_right(
    pair: pest::iterators::Pair<Rule>,
) -> Result<ComparisonRight, ReaperError> {
    // The pair is comparison_right, which contains entity_attr, var_attr, value, or ident
    let inner = pair
        .into_inner()
        .next()
        .ok_or_else(|| ReaperError::InvalidPolicy {
            reason: "Empty comparison right".to_string(),
        })?;

    match inner.as_rule() {
        Rule::entity_attr => Ok(ComparisonRight::EntityAttr(parse_entity_attr(inner)?)),
        Rule::var_attr => Ok(ComparisonRight::VarAttr(parse_var_attr(inner)?)),
        Rule::value => Ok(ComparisonRight::Value(parse_value(inner)?)),
        Rule::ident => Ok(ComparisonRight::Variable(inner.as_str().to_string())),
        _ => Err(ReaperError::InvalidPolicy {
            reason: format!("Unexpected comparison right type: {:?}", inner.as_rule()),
        }),
    }
}

fn parse_entity_attr(pair: pest::iterators::Pair<Rule>) -> Result<EntityAttr, ReaperError> {
    let mut inner = pair.into_inner();
    let entity = Entity::from(inner.next().unwrap().as_str());
    let attribute = inner.next().unwrap().as_str().to_string();

    // Check for optional bracket index
    let index = if let Some(bracket_pair) = inner.next() {
        if bracket_pair.as_rule() == Rule::bracket_index {
            let index_value_pair = bracket_pair.into_inner().next().unwrap();
            Some(parse_bracket_index(index_value_pair)?)
        } else {
            None
        }
    } else {
        None
    };

    Ok(EntityAttr {
        entity,
        attribute,
        index,
    })
}

fn parse_var_attr(pair: pest::iterators::Pair<Rule>) -> Result<VarAttr, ReaperError> {
    let mut inner = pair.into_inner();
    let variable = inner.next().unwrap().as_str().to_string();
    let attribute = inner.next().unwrap().as_str().to_string();

    // Check for optional bracket index
    let index = if let Some(bracket_pair) = inner.next() {
        if bracket_pair.as_rule() == Rule::bracket_index {
            let index_value_pair = bracket_pair.into_inner().next().unwrap();
            Some(parse_bracket_index(index_value_pair)?)
        } else {
            None
        }
    } else {
        None
    };

    Ok(VarAttr {
        variable,
        attribute,
        index,
    })
}

fn parse_bracket_index(pair: pest::iterators::Pair<Rule>) -> Result<Index, ReaperError> {
    // The pair is bracket_index_value, which contains either "_", integer, or string
    // Check for wildcard first (literal "_")
    if pair.as_str() == "_" {
        return Ok(Index::Wildcard);
    }

    let inner = pair
        .into_inner()
        .next()
        .ok_or_else(|| ReaperError::InvalidPolicy {
            reason: "Empty bracket index".to_string(),
        })?;

    match inner.as_rule() {
        Rule::integer => {
            let val = inner
                .as_str()
                .parse::<i64>()
                .map_err(|e| ReaperError::InvalidPolicy {
                    reason: format!("Invalid integer index: {}", e),
                })?;
            Ok(Index::Number(val))
        }
        Rule::string => {
            let s = parse_string_literal(inner)?;
            Ok(Index::String(s))
        }
        _ => Err(ReaperError::InvalidPolicy {
            reason: format!("Unexpected bracket index type: {:?}", inner.as_rule()),
        }),
    }
}

fn parse_value(pair: pest::iterators::Pair<Rule>) -> Result<Value, ReaperError> {
    let inner = pair.into_inner().next().unwrap();

    match inner.as_rule() {
        Rule::string => Ok(Value::String(parse_string_literal(inner)?)),
        Rule::integer => {
            let val = inner
                .as_str()
                .parse::<i64>()
                .map_err(|e| ReaperError::InvalidPolicy {
                    reason: format!("Invalid integer: {}", e),
                })?;
            Ok(Value::Integer(val))
        }
        Rule::float => {
            let val = inner
                .as_str()
                .parse::<f64>()
                .map_err(|e| ReaperError::InvalidPolicy {
                    reason: format!("Invalid float: {}", e),
                })?;
            Ok(Value::Float(val))
        }
        Rule::boolean_literal => {
            let val = inner.as_str() == "true";
            Ok(Value::Boolean(val))
        }
        Rule::null_literal => Ok(Value::Null),
        Rule::array => parse_array(inner),
        Rule::braced_expr => parse_braced_expr(inner),
        _ => Err(ReaperError::InvalidPolicy {
            reason: format!("Unexpected value type: {:?}", inner.as_rule()),
        }),
    }
}

fn parse_array(pair: pest::iterators::Pair<Rule>) -> Result<Value, ReaperError> {
    let mut values = Vec::new();

    for inner in pair.into_inner() {
        if inner.as_rule() == Rule::value {
            values.push(parse_value(inner)?);
        }
    }

    Ok(Value::Array(values))
}

fn parse_braced_expr(pair: pest::iterators::Pair<Rule>) -> Result<Value, ReaperError> {
    let mut has_pairs = false;
    let mut has_values = false;
    let mut object_pairs = Vec::new();
    let mut set_values = Vec::new();

    for inner in pair.into_inner() {
        if inner.as_rule() == Rule::braced_items {
            for item in inner.into_inner() {
                if item.as_rule() == Rule::braced_item {
                    let content = item.into_inner().next().unwrap();

                    match content.as_rule() {
                        Rule::object_pair => {
                            has_pairs = true;
                            let mut pair_inner = content.into_inner();

                            // First element is either string or ident
                            let key_pair = pair_inner.next().unwrap();
                            let key = match key_pair.as_rule() {
                                Rule::string => parse_string_literal(key_pair)?,
                                Rule::ident => key_pair.as_str().to_string(),
                                _ => {
                                    return Err(ReaperError::InvalidPolicy {
                                        reason: format!(
                                            "Unexpected key type: {:?}",
                                            key_pair.as_rule()
                                        ),
                                    })
                                }
                            };

                            // Second element is the value
                            let value = parse_value(pair_inner.next().unwrap())?;
                            object_pairs.push((key, value));
                        }
                        Rule::value => {
                            has_values = true;
                            set_values.push(parse_value(content)?);
                        }
                        _ => {
                            return Err(ReaperError::InvalidPolicy {
                                reason: format!("Unexpected braced item: {:?}", content.as_rule()),
                            })
                        }
                    }
                }
            }
        }
    }

    // Validate: can't mix object pairs and values
    if has_pairs && has_values {
        return Err(ReaperError::InvalidPolicy {
            reason: "Cannot mix object pairs (key: value) and set values in same braced expression"
                .to_string(),
        });
    }

    if has_pairs {
        Ok(Value::Object(object_pairs))
    } else {
        // Empty {} or set
        Ok(Value::Set(set_values))
    }
}

fn parse_string_literal(pair: pest::iterators::Pair<Rule>) -> Result<String, ReaperError> {
    // String is atomic (@), so we get the full string with quotes
    let s = pair.as_str();
    // Remove surrounding quotes
    let trimmed = &s[1..s.len() - 1];
    // Unescape if needed (simple implementation)
    Ok(trimmed.replace("\\\"", "\"").replace("\\\\", "\\"))
}

/// Parse a comprehension expression
/// Comprehensions collect and transform data from collections
fn parse_comprehension(pair: pest::iterators::Pair<Rule>) -> Result<Comprehension, ReaperError> {
    // Comprehension contains either set_comprehension, array_comprehension, or object_comprehension
    let inner = pair
        .into_inner()
        .next()
        .ok_or_else(|| ReaperError::InvalidPolicy {
            reason: "Empty comprehension".to_string(),
        })?;

    match inner.as_rule() {
        Rule::set_comprehension => parse_set_comprehension(inner),
        Rule::array_comprehension => parse_array_comprehension(inner),
        Rule::object_comprehension => parse_object_comprehension(inner),
        _ => Err(ReaperError::InvalidPolicy {
            reason: format!("Unexpected comprehension type: {:?}", inner.as_rule()),
        }),
    }
}

/// Parse set comprehension: {expr | iteration; filters}
fn parse_set_comprehension(
    pair: pest::iterators::Pair<Rule>,
) -> Result<Comprehension, ReaperError> {
    let mut output: Option<Box<Expr>> = None;
    let mut iterator: Option<ComprehensionIterator> = None;
    let mut filters: Vec<Condition> = Vec::new();

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::comp_expr => {
                output = Some(Box::new(parse_comp_expr(inner)?));
            }
            Rule::comp_iterator => {
                iterator = Some(parse_comp_iterator(inner)?);
            }
            Rule::comp_filters => {
                filters = parse_comp_filters(inner)?;
            }
            _ => {}
        }
    }

    Ok(Comprehension::Set {
        output: output.ok_or_else(|| ReaperError::InvalidPolicy {
            reason: "Set comprehension missing output expression".to_string(),
        })?,
        iterator: iterator.ok_or_else(|| ReaperError::InvalidPolicy {
            reason: "Set comprehension missing iterator".to_string(),
        })?,
        filters,
    })
}

/// Parse array comprehension: [expr | iteration; filters]
fn parse_array_comprehension(
    pair: pest::iterators::Pair<Rule>,
) -> Result<Comprehension, ReaperError> {
    let mut output: Option<Box<Expr>> = None;
    let mut iterator: Option<ComprehensionIterator> = None;
    let mut filters: Vec<Condition> = Vec::new();

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::comp_expr => {
                output = Some(Box::new(parse_comp_expr(inner)?));
            }
            Rule::comp_iterator => {
                iterator = Some(parse_comp_iterator(inner)?);
            }
            Rule::comp_filters => {
                filters = parse_comp_filters(inner)?;
            }
            _ => {}
        }
    }

    Ok(Comprehension::Array {
        output: output.ok_or_else(|| ReaperError::InvalidPolicy {
            reason: "Array comprehension missing output expression".to_string(),
        })?,
        iterator: iterator.ok_or_else(|| ReaperError::InvalidPolicy {
            reason: "Array comprehension missing iterator".to_string(),
        })?,
        filters,
    })
}

/// Parse object comprehension: {key: value | iteration; filters}
fn parse_object_comprehension(
    pair: pest::iterators::Pair<Rule>,
) -> Result<Comprehension, ReaperError> {
    let mut key: Option<Box<Expr>> = None;
    let mut value: Option<Box<Expr>> = None;
    let mut iterator: Option<ComprehensionIterator> = None;
    let mut filters: Vec<Condition> = Vec::new();

    let mut expr_count = 0;
    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::comp_expr => {
                if expr_count == 0 {
                    key = Some(Box::new(parse_comp_expr(inner)?));
                } else if expr_count == 1 {
                    value = Some(Box::new(parse_comp_expr(inner)?));
                }
                expr_count += 1;
            }
            Rule::comp_iterator => {
                iterator = Some(parse_comp_iterator(inner)?);
            }
            Rule::comp_filters => {
                filters = parse_comp_filters(inner)?;
            }
            _ => {}
        }
    }

    Ok(Comprehension::Object {
        key: key.ok_or_else(|| ReaperError::InvalidPolicy {
            reason: "Object comprehension missing key expression".to_string(),
        })?,
        value: value.ok_or_else(|| ReaperError::InvalidPolicy {
            reason: "Object comprehension missing value expression".to_string(),
        })?,
        iterator: iterator.ok_or_else(|| ReaperError::InvalidPolicy {
            reason: "Object comprehension missing iterator".to_string(),
        })?,
        filters,
    })
}

/// Parse comprehension iterator: u := users[_]
fn parse_comp_iterator(
    pair: pest::iterators::Pair<Rule>,
) -> Result<ComprehensionIterator, ReaperError> {
    let mut variable: Option<String> = None;
    let mut collection: Option<EntityAttr> = None;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::ident => {
                variable = Some(inner.as_str().to_string());
            }
            Rule::entity_attr => {
                collection = Some(parse_entity_attr(inner)?);
            }
            _ => {}
        }
    }

    Ok(ComprehensionIterator {
        variable: variable.ok_or_else(|| ReaperError::InvalidPolicy {
            reason: "Iterator missing variable name".to_string(),
        })?,
        collection: collection.ok_or_else(|| ReaperError::InvalidPolicy {
            reason: "Iterator missing collection".to_string(),
        })?,
    })
}

/// Parse comprehension filters: ; condition ; condition ...
fn parse_comp_filters(pair: pest::iterators::Pair<Rule>) -> Result<Vec<Condition>, ReaperError> {
    let mut filters = Vec::new();

    for inner in pair.into_inner() {
        if inner.as_rule() == Rule::condition {
            filters.push(parse_condition(inner)?);
        }
    }

    Ok(filters)
}

/// Parse comprehension output expression
fn parse_comp_expr(pair: pest::iterators::Pair<Rule>) -> Result<Expr, ReaperError> {
    let mut inner_pairs = pair.into_inner();
    let first = inner_pairs
        .next()
        .ok_or_else(|| ReaperError::InvalidPolicy {
            reason: "Empty comprehension expression".to_string(),
        })?;

    match first.as_rule() {
        Rule::comp_function_call => parse_comp_function_call(first),

        // Method calls or attribute access: perms.count(), u.name.lower(), or just u.name
        Rule::comp_method_or_access => {
            let mut inner = first.into_inner();
            let first_inner = inner.next().ok_or_else(|| ReaperError::InvalidPolicy {
                reason: "Empty method or access expression".to_string(),
            })?;

            match first_inner.as_rule() {
                // u.name or u.name.method() - dot access with optional methods
                Rule::comp_dot_access_with_methods => {
                    let dot_inner = first_inner.into_inner();
                    let mut variable: Option<String> = None;
                    let mut attribute: Option<String> = None;
                    let mut index: Option<Index> = None;
                    let mut method_chain: Option<pest::iterators::Pair<Rule>> = None;
                    let mut ident_count = 0;

                    for part in dot_inner {
                        match part.as_rule() {
                            Rule::ident => {
                                if ident_count == 0 {
                                    variable = Some(part.as_str().to_string());
                                } else if ident_count == 1 {
                                    attribute = Some(part.as_str().to_string());
                                }
                                ident_count += 1;
                            }
                            Rule::bracket_index => {
                                let idx_value = part.into_inner().next().ok_or_else(|| {
                                    ReaperError::InvalidPolicy {
                                        reason: "Empty bracket index".to_string(),
                                    }
                                })?;
                                index = Some(parse_bracket_index(idx_value)?);
                            }
                            Rule::comp_method_chain => {
                                method_chain = Some(part);
                            }
                            _ => {}
                        }
                    }

                    // Build base expression (attribute access or indexed access)
                    let base = if let Some(idx) = index {
                        Expr::IndexedAccess {
                            variable: variable.ok_or_else(|| ReaperError::InvalidPolicy {
                                reason: "Missing variable in indexed access".to_string(),
                            })?,
                            attribute: attribute.ok_or_else(|| ReaperError::InvalidPolicy {
                                reason: "Missing attribute in indexed access".to_string(),
                            })?,
                            index: idx,
                        }
                    } else {
                        Expr::AttributeAccess {
                            variable: variable.ok_or_else(|| ReaperError::InvalidPolicy {
                                reason: "Missing variable in attribute access".to_string(),
                            })?,
                            attribute: attribute.ok_or_else(|| ReaperError::InvalidPolicy {
                                reason: "Missing attribute in attribute access".to_string(),
                            })?,
                        }
                    };

                    // Apply method chain if present
                    if let Some(chain) = method_chain {
                        parse_comp_method_chain(base, chain)
                    } else {
                        Ok(base)
                    }
                }

                // var or var.method() - simple variable with optional method chain
                Rule::ident => {
                    let variable = first_inner.as_str().to_string();
                    let base = Expr::Variable(variable);

                    // Check if there's an optional method chain
                    if let Some(method_chain) = inner.next() {
                        if method_chain.as_rule() == Rule::comp_method_chain {
                            parse_comp_method_chain(base, method_chain)
                        } else {
                            Err(ReaperError::InvalidPolicy {
                                reason: format!(
                                    "Expected method chain, got {:?}",
                                    method_chain.as_rule()
                                ),
                            })
                        }
                    } else {
                        Ok(base)
                    }
                }

                _ => Err(ReaperError::InvalidPolicy {
                    reason: format!(
                        "Unexpected rule in method_or_access: {:?}",
                        first_inner.as_rule()
                    ),
                }),
            }
        }

        // Literals
        Rule::value => Ok(Expr::Literal(parse_value(first)?)),

        // Base expressions without method calls (backward compatibility)
        Rule::comp_base_expr => parse_comp_base_expr(first),

        _ => Err(ReaperError::InvalidPolicy {
            reason: format!(
                "Unexpected comprehension expression type: {:?}",
                first.as_rule()
            ),
        }),
    }
}

/// Parse base expression that can be a receiver for method calls
fn parse_comp_base_expr(pair: pest::iterators::Pair<Rule>) -> Result<Expr, ReaperError> {
    let inner = pair
        .into_inner()
        .next()
        .ok_or_else(|| ReaperError::InvalidPolicy {
            reason: "Empty base expression".to_string(),
        })?;

    match inner.as_rule() {
        Rule::comp_dot_access => parse_comp_dot_access(inner),
        Rule::comp_variable => Ok(Expr::Variable(inner.as_str().to_string())),
        Rule::value => Ok(Expr::Literal(parse_value(inner)?)),
        _ => Err(ReaperError::InvalidPolicy {
            reason: format!("Unexpected base expression type: {:?}", inner.as_rule()),
        }),
    }
}

/// Parse dot access: u.name or u.roles[0]
fn parse_comp_dot_access(pair: pest::iterators::Pair<Rule>) -> Result<Expr, ReaperError> {
    let mut variable: Option<String> = None;
    let mut attribute: Option<String> = None;
    let mut index: Option<Index> = None;

    let mut ident_count = 0;
    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::ident => {
                if ident_count == 0 {
                    variable = Some(inner.as_str().to_string());
                } else if ident_count == 1 {
                    attribute = Some(inner.as_str().to_string());
                }
                ident_count += 1;
            }
            Rule::bracket_index => {
                let idx_value =
                    inner
                        .into_inner()
                        .next()
                        .ok_or_else(|| ReaperError::InvalidPolicy {
                            reason: "Empty bracket index".to_string(),
                        })?;
                index = Some(parse_bracket_index(idx_value)?);
            }
            _ => {}
        }
    }

    let var = variable.ok_or_else(|| ReaperError::InvalidPolicy {
        reason: "Dot access missing variable".to_string(),
    })?;
    let attr = attribute.ok_or_else(|| ReaperError::InvalidPolicy {
        reason: "Dot access missing attribute".to_string(),
    })?;

    if let Some(idx) = index {
        Ok(Expr::IndexedAccess {
            variable: var,
            attribute: attr,
            index: idx,
        })
    } else {
        Ok(Expr::AttributeAccess {
            variable: var,
            attribute: attr,
        })
    }
}

/// Parse method chain: .method1().method2()...
fn parse_comp_method_chain(
    mut receiver: Expr,
    chain_pair: pest::iterators::Pair<Rule>,
) -> Result<Expr, ReaperError> {
    for method_call in chain_pair.into_inner() {
        if method_call.as_rule() != Rule::comp_single_method_call {
            continue;
        }

        let mut method_name: Option<String> = None;
        let mut args: Vec<Expr> = Vec::new();

        for inner in method_call.into_inner() {
            match inner.as_rule() {
                Rule::ident => {
                    method_name = Some(inner.as_str().to_string());
                }
                Rule::comp_arg_list => {
                    for arg in inner.into_inner() {
                        args.push(parse_comp_expr(arg)?);
                    }
                }
                _ => {}
            }
        }

        let method =
            MethodName::from_str(&method_name.ok_or_else(|| ReaperError::InvalidPolicy {
                reason: "Method call missing method name".to_string(),
            })?)
            .map_err(|e| ReaperError::InvalidPolicy { reason: e })?;

        receiver = Expr::MethodCall {
            receiver: Box::new(receiver),
            method,
            args,
        };
    }

    Ok(receiver)
}

/// Parse function call in comprehension: is_string(x), concat(a, b), time.now_ns()
fn parse_comp_function_call(pair: pest::iterators::Pair<Rule>) -> Result<Expr, ReaperError> {
    let mut namespace: Option<String> = None;
    let mut function_name: Option<String> = None;
    let mut args: Vec<Expr> = Vec::new();
    let mut ident_count = 0;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::ident => {
                // First ident could be namespace or function name
                // Second ident is function name if there's a namespace
                if ident_count == 0 {
                    // Store as potential function name
                    function_name = Some(inner.as_str().to_string());
                } else if ident_count == 1 {
                    // First ident was namespace
                    namespace = function_name.clone();
                    function_name = Some(inner.as_str().to_string());
                }
                ident_count += 1;
            }
            Rule::comp_arg_list => {
                // Parse arguments
                for arg in inner.into_inner() {
                    args.push(parse_comp_expr(arg)?);
                }
            }
            _ => {}
        }
    }

    Ok(Expr::FunctionCall {
        namespace,
        function: function_name.ok_or_else(|| ReaperError::InvalidPolicy {
            reason: "Function call missing function name".to_string(),
        })?,
        args,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_policy() {
        let input = r#"
            policy test {
                default: deny,
                rule admin { allow if user.role == "admin" }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        assert_eq!(policy.name, "test");
        assert_eq!(policy.rules.len(), 1);
        assert_eq!(policy.rules[0].name, "admin");
    }

    #[test]
    fn test_parse_with_metadata() {
        let input = r#"
            policy test {
                version: "1.0.0",
                description: "Test policy",
                default: allow,
                rule test { deny if user.suspended == true }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        assert_eq!(policy.metadata.get("version"), Some(&"1.0.0".to_string()));
    }

    #[test]
    fn test_parse_complex_condition() {
        let input = r#"
            policy test {
                default: deny,
                rule complex {
                    allow if {
                        user.department == resource.department &&
                        user.clearance >= resource.clearance_required
                    }
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        assert_eq!(policy.rules.len(), 1);
    }

    #[test]
    fn test_parse_array_values() {
        let input = r#"
            policy test {
                default: deny,
                rule array_test {
                    allow if user.roles == [1, 2, 3]
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        assert_eq!(policy.rules.len(), 1);

        // Verify it's a comparison with an array value
        if let Condition::Comparison {
            right: ComparisonRight::Value(Value::Array(arr)),
            ..
        } = &policy.rules[0].condition
        {
            assert_eq!(arr.len(), 3);
        } else {
            panic!("Expected array value");
        }
    }

    #[test]
    fn test_parse_empty_array() {
        let input = r#"
            policy test {
                default: deny,
                rule empty_array { allow if user.items == [] }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Comparison {
            right: ComparisonRight::Value(Value::Array(arr)),
            ..
        } = &policy.rules[0].condition
        {
            assert_eq!(arr.len(), 0);
        } else {
            panic!("Expected empty array");
        }
    }

    #[test]
    fn test_parse_nested_array() {
        let input = r#"
            policy test {
                default: deny,
                rule nested { allow if user.matrix == [[1, 2], [3, 4]] }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Comparison {
            right: ComparisonRight::Value(Value::Array(arr)),
            ..
        } = &policy.rules[0].condition
        {
            assert_eq!(arr.len(), 2);
            if let Value::Array(inner) = &arr[0] {
                assert_eq!(inner.len(), 2);
            } else {
                panic!("Expected nested array");
            }
        } else {
            panic!("Expected array value");
        }
    }

    #[test]
    fn test_parse_object_values() {
        let input = r#"
            policy test {
                default: deny,
                rule object_test {
                    allow if user.config == {"timeout": 30, "retries": 3}
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Comparison {
            right: ComparisonRight::Value(Value::Object(obj)),
            ..
        } = &policy.rules[0].condition
        {
            assert_eq!(obj.len(), 2);
            assert_eq!(obj[0].0, "timeout");
            if let Value::Integer(val) = obj[0].1 {
                assert_eq!(val, 30);
            } else {
                panic!("Expected integer value");
            }
        } else {
            panic!("Expected object value");
        }
    }

    #[test]
    fn test_parse_set_values() {
        let input = r#"
            policy test {
                default: deny,
                rule set_test {
                    allow if user.permissions == {"read", "write", "delete"}
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Comparison {
            right: ComparisonRight::Value(Value::Set(set)),
            ..
        } = &policy.rules[0].condition
        {
            assert_eq!(set.len(), 3);
        } else {
            panic!("Expected set value");
        }
    }

    #[test]
    fn test_parse_empty_set() {
        let input = r#"
            policy test {
                default: deny,
                rule empty_set { allow if user.tags == {} }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Comparison {
            right: ComparisonRight::Value(Value::Set(set)),
            ..
        } = &policy.rules[0].condition
        {
            assert_eq!(set.len(), 0);
        } else {
            panic!("Expected empty set");
        }
    }

    #[test]
    fn test_parse_nested_object() {
        let input = r#"
            policy test {
                default: deny,
                rule nested {
                    allow if user.profile == {"name": "alice", "settings": {"theme": "dark"}}
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Comparison {
            right: ComparisonRight::Value(Value::Object(obj)),
            ..
        } = &policy.rules[0].condition
        {
            assert_eq!(obj.len(), 2);
            if let Value::Object(inner) = &obj[1].1 {
                assert_eq!(inner.len(), 1);
                assert_eq!(inner[0].0, "theme");
            } else {
                panic!("Expected nested object");
            }
        } else {
            panic!("Expected object value");
        }
    }

    #[test]
    fn test_parse_mixed_types_in_array() {
        let input = r#"
            policy test {
                default: deny,
                rule mixed {
                    allow if user.data == [1, "hello", true, null, [2, 3]]
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Comparison {
            right: ComparisonRight::Value(Value::Array(arr)),
            ..
        } = &policy.rules[0].condition
        {
            assert_eq!(arr.len(), 5);
            assert!(matches!(arr[0], Value::Integer(_)));
            assert!(matches!(arr[1], Value::String(_)));
            assert!(matches!(arr[2], Value::Boolean(_)));
            assert!(matches!(arr[3], Value::Null));
            assert!(matches!(arr[4], Value::Array(_)));
        } else {
            panic!("Expected array with mixed types");
        }
    }

    #[test]
    fn test_parse_bracket_notation_numeric() {
        let input = r#"
            policy test {
                default: deny,
                rule array_index {
                    allow if user.roles[0] == "admin"
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Comparison { left, .. } = &policy.rules[0].condition {
            if let ComparisonLeft::EntityAttr(attr) = left {
                assert_eq!(attr.attribute, "roles");
                assert!(attr.index.is_some());
                if let Some(Index::Number(n)) = &attr.index {
                    assert_eq!(n, &0);
                } else {
                    panic!("Expected numeric index");
                }
            } else {
                panic!("Expected entity attribute");
            }
        } else {
            panic!("Expected comparison");
        }
    }

    #[test]
    fn test_parse_bracket_notation_string() {
        let input = r#"
            policy test {
                default: deny,
                rule object_key {
                    allow if user.data["department"] == "engineering"
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Comparison { left, .. } = &policy.rules[0].condition {
            if let ComparisonLeft::EntityAttr(attr) = left {
                assert_eq!(attr.attribute, "data");
                assert!(attr.index.is_some());
                if let Some(Index::String(s)) = &attr.index {
                    assert_eq!(s, "department");
                } else {
                    panic!("Expected string index");
                }
            } else {
                panic!("Expected entity attribute");
            }
        } else {
            panic!("Expected comparison");
        }
    }

    #[test]
    fn test_parse_in_operator() {
        let input = r#"
            policy test {
                default: deny,
                rule membership {
                    allow if "admin" in user.roles
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        // "admin" in user.roles is parsed as: left=user.roles, op=In, right="admin"
        if let Condition::Comparison { left, op, right } = &policy.rules[0].condition {
            assert_eq!(*op, Operator::In);
            if let ComparisonLeft::EntityAttr(attr) = left {
                assert_eq!(attr.entity, Entity::User);
                assert_eq!(attr.attribute, "roles");
            } else {
                panic!("Expected entity attribute");
            }
            if let ComparisonRight::Value(Value::String(s)) = right {
                assert_eq!(s, "admin");
            } else {
                panic!("Expected string value on right side");
            }
        } else {
            panic!("Expected comparison");
        }
    }

    #[test]
    fn test_parse_in_operator_with_variable() {
        let input = r#"
            policy test {
                default: deny,
                rule check_permission {
                    allow if context.action in resource.allowed_actions
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Comparison { left, op, right } = &policy.rules[0].condition {
            assert_eq!(*op, Operator::In);
            if let ComparisonLeft::EntityAttr(attr) = left {
                assert_eq!(attr.entity, Entity::Context);
                assert_eq!(attr.attribute, "action");
            } else {
                panic!("Expected entity attribute on left side");
            }
            if let ComparisonRight::EntityAttr(attr) = right {
                assert_eq!(attr.entity, Entity::Resource);
                assert_eq!(attr.attribute, "allowed_actions");
            } else {
                panic!("Expected entity attribute on right side");
            }
        } else {
            panic!("Expected comparison");
        }
    }

    #[test]
    fn test_parse_variable_assignment() {
        let input = r#"
            policy test {
                default: deny,
                rule with_variable {
                    allow if role := user.role
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        // The condition should be a simple assignment
        if let Condition::Assignment { variable, value } = &policy.rules[0].condition {
            assert_eq!(variable, "role");
            if let AssignmentValue::EntityAttr(attr) = value {
                assert_eq!(attr.entity, Entity::User);
                assert_eq!(attr.attribute, "role");
            } else {
                panic!("Expected entity attr in assignment");
            }
        } else {
            panic!("Expected assignment");
        }
    }

    #[test]
    fn test_parse_assignment_value_types() {
        // Test assignment from literal value
        let input = r#"
            policy test {
                default: deny,
                rule literal_assign {
                    allow if x := "admin"
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Assignment { variable, value } = &policy.rules[0].condition {
            assert_eq!(variable, "x");
            if let AssignmentValue::Value(Value::String(s)) = value {
                assert_eq!(s, "admin");
            } else {
                panic!("Expected string value");
            }
        } else {
            panic!("Expected assignment");
        }
    }

    #[test]
    fn test_parse_comparison_with_variable_right() {
        let input = r#"
            policy test {
                default: deny,
                rule compare_var {
                    allow if user.role == role_var
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Comparison { left, op, right } = &policy.rules[0].condition {
            if let ComparisonLeft::EntityAttr(attr) = left {
                assert_eq!(attr.entity, Entity::User);
                assert_eq!(attr.attribute, "role");
            } else {
                panic!("Expected entity attribute on left side");
            }
            assert_eq!(*op, Operator::Equal);
            if let ComparisonRight::Variable(var) = right {
                assert_eq!(var, "role_var");
            } else {
                panic!("Expected variable on right side");
            }
        } else {
            panic!("Expected comparison");
        }
    }

    // ========== COMPREHENSION PARSER TESTS ==========

    #[test]
    fn test_parse_set_comprehension_simple() {
        let input = r#"
            policy test {
                default: deny,
                rule collect_names {
                    allow if admin_names := {u.name | u := user.team[_]}
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Assignment { variable, value } = &policy.rules[0].condition {
            assert_eq!(variable, "admin_names");
            if let AssignmentValue::Comprehension(Comprehension::Set {
                output,
                iterator,
                filters,
            }) = value
            {
                // Check output expression: u.name
                if let Expr::AttributeAccess {
                    variable: var,
                    attribute: attr,
                } = output.as_ref()
                {
                    assert_eq!(var, "u");
                    assert_eq!(attr, "name");
                } else {
                    panic!("Expected attribute access in output");
                }

                // Check iterator: u := user.team[_]
                assert_eq!(iterator.variable, "u");
                assert_eq!(iterator.collection.entity, Entity::User);
                assert_eq!(iterator.collection.attribute, "team");
                assert!(matches!(iterator.collection.index, Some(Index::Wildcard)));

                // No filters
                assert_eq!(filters.len(), 0);
            } else {
                panic!("Expected set comprehension");
            }
        } else {
            panic!("Expected assignment");
        }
    }

    #[test]
    fn test_parse_array_comprehension_simple() {
        let input = r#"
            policy test {
                default: deny,
                rule collect_emails {
                    allow if all_emails := [u.email | u := user.contacts[_]]
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Assignment { variable, value } = &policy.rules[0].condition {
            assert_eq!(variable, "all_emails");
            if let AssignmentValue::Comprehension(Comprehension::Array {
                output,
                iterator,
                filters,
            }) = value
            {
                // Check output expression: u.email
                if let Expr::AttributeAccess {
                    variable: var,
                    attribute: attr,
                } = output.as_ref()
                {
                    assert_eq!(var, "u");
                    assert_eq!(attr, "email");
                } else {
                    panic!("Expected attribute access in output");
                }

                // Check iterator
                assert_eq!(iterator.variable, "u");
                assert_eq!(iterator.collection.entity, Entity::User);
                assert_eq!(iterator.collection.attribute, "contacts");

                // No filters
                assert_eq!(filters.len(), 0);
            } else {
                panic!("Expected array comprehension");
            }
        } else {
            panic!("Expected assignment");
        }
    }

    #[test]
    fn test_parse_object_comprehension_simple() {
        let input = r#"
            policy test {
                default: deny,
                rule create_user_map {
                    allow if user_map := {u.id: u.name | u := user.all_users[_]}
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Assignment { variable, value } = &policy.rules[0].condition {
            assert_eq!(variable, "user_map");
            if let AssignmentValue::Comprehension(Comprehension::Object {
                key,
                value: val,
                iterator,
                filters,
            }) = value
            {
                // Check key expression: u.id
                if let Expr::AttributeAccess {
                    variable: var,
                    attribute: attr,
                } = key.as_ref()
                {
                    assert_eq!(var, "u");
                    assert_eq!(attr, "id");
                } else {
                    panic!("Expected attribute access in key");
                }

                // Check value expression: u.name
                if let Expr::AttributeAccess {
                    variable: var,
                    attribute: attr,
                } = val.as_ref()
                {
                    assert_eq!(var, "u");
                    assert_eq!(attr, "name");
                } else {
                    panic!("Expected attribute access in value");
                }

                // Check iterator
                assert_eq!(iterator.variable, "u");
                assert_eq!(iterator.collection.entity, Entity::User);

                // No filters
                assert_eq!(filters.len(), 0);
            } else {
                panic!("Expected object comprehension");
            }
        } else {
            panic!("Expected assignment");
        }
    }

    #[test]
    fn test_parse_comprehension_with_single_filter() {
        let input = r#"
            policy test {
                default: deny,
                rule active_users {
                    allow if active := {u.name | u := user.users[_]; u.active == true}
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Assignment { variable, value } = &policy.rules[0].condition {
            assert_eq!(variable, "active");
            if let AssignmentValue::Comprehension(Comprehension::Set {
                output: _,
                iterator,
                filters,
            }) = value
            {
                assert_eq!(iterator.variable, "u");
                assert_eq!(filters.len(), 1);

                // Check filter: u.active == true
                if let Condition::Comparison { left, op, right } = &filters[0] {
                    if let ComparisonLeft::VarAttr(var_attr) = left {
                        assert_eq!(var_attr.variable, "u");
                        assert_eq!(var_attr.attribute, "active");
                    } else {
                        panic!("Expected var attribute in filter");
                    }
                    assert_eq!(*op, Operator::Equal);
                    if let ComparisonRight::Value(Value::Boolean(b)) = right {
                        assert!(*b);
                    } else {
                        panic!("Expected boolean value");
                    }
                } else {
                    panic!("Expected comparison in filter");
                }
            } else {
                panic!("Expected set comprehension");
            }
        } else {
            panic!("Expected assignment");
        }
    }

    #[test]
    fn test_parse_comprehension_with_multiple_filters() {
        let input = r#"
            policy test {
                default: deny,
                rule senior_devs {
                    allow if senior_dev_emails := [u.email |
                        u := user.employees[_];
                        u.role == "developer";
                        u.years_experience >= 5
                    ]
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Assignment { variable, value } = &policy.rules[0].condition {
            assert_eq!(variable, "senior_dev_emails");
            if let AssignmentValue::Comprehension(Comprehension::Array {
                output,
                iterator,
                filters,
            }) = value
            {
                // Check output: u.email
                if let Expr::AttributeAccess {
                    variable: var,
                    attribute: attr,
                } = output.as_ref()
                {
                    assert_eq!(var, "u");
                    assert_eq!(attr, "email");
                } else {
                    panic!("Expected attribute access");
                }

                // Check iterator
                assert_eq!(iterator.variable, "u");

                // Check two filters
                assert_eq!(filters.len(), 2);

                // Filter 1: u.role == "developer"
                if let Condition::Comparison { left, op, right } = &filters[0] {
                    if let ComparisonLeft::VarAttr(var_attr) = left {
                        assert_eq!(var_attr.variable, "u");
                        assert_eq!(var_attr.attribute, "role");
                    } else {
                        panic!("Expected var attribute in first filter");
                    }
                    assert_eq!(*op, Operator::Equal);
                    if let ComparisonRight::Value(Value::String(s)) = right {
                        assert_eq!(s, "developer");
                    } else {
                        panic!("Expected string value");
                    }
                } else {
                    panic!("Expected comparison in first filter");
                }

                // Filter 2: u.years_experience >= 5
                if let Condition::Comparison { left, op, right } = &filters[1] {
                    if let ComparisonLeft::VarAttr(var_attr) = left {
                        assert_eq!(var_attr.variable, "u");
                        assert_eq!(var_attr.attribute, "years_experience");
                    } else {
                        panic!("Expected var attribute in second filter");
                    }
                    assert_eq!(*op, Operator::GreaterEqual);
                    if let ComparisonRight::Value(Value::Integer(i)) = right {
                        assert_eq!(*i, 5);
                    } else {
                        panic!("Expected integer value");
                    }
                } else {
                    panic!("Expected comparison in second filter");
                }
            } else {
                panic!("Expected array comprehension");
            }
        } else {
            panic!("Expected assignment");
        }
    }

    #[test]
    fn test_parse_comprehension_with_literal_output() {
        let input = r#"
            policy test {
                default: deny,
                rule count_users {
                    allow if counts := [1 | u := user.users[_]]
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Assignment { variable, value } = &policy.rules[0].condition {
            assert_eq!(variable, "counts");
            if let AssignmentValue::Comprehension(Comprehension::Array {
                output,
                iterator: _,
                filters: _,
            }) = value
            {
                // Check output: literal 1
                if let Expr::Literal(Value::Integer(i)) = output.as_ref() {
                    assert_eq!(*i, 1);
                } else {
                    panic!("Expected literal integer in output");
                }
            } else {
                panic!("Expected array comprehension");
            }
        } else {
            panic!("Expected assignment");
        }
    }

    #[test]
    fn test_parse_comprehension_with_variable_output() {
        let input = r#"
            policy test {
                default: deny,
                rule collect_vars {
                    allow if collected := {u | u := user.items[_]}
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Assignment { variable, value } = &policy.rules[0].condition {
            assert_eq!(variable, "collected");
            if let AssignmentValue::Comprehension(Comprehension::Set {
                output,
                iterator,
                filters: _,
            }) = value
            {
                // Check output: variable u
                if let Expr::Variable(var) = output.as_ref() {
                    assert_eq!(var, "u");
                    assert_eq!(var, &iterator.variable); // Same as iterator variable
                } else {
                    panic!("Expected variable in output");
                }
            } else {
                panic!("Expected set comprehension");
            }
        } else {
            panic!("Expected assignment");
        }
    }

    #[test]
    fn test_parse_comprehension_with_indexed_output() {
        let input = r#"
            policy test {
                default: deny,
                rule first_roles {
                    allow if first_roles := [u.roles[0] | u := user.users[_]]
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Assignment { variable, value } = &policy.rules[0].condition {
            assert_eq!(variable, "first_roles");
            if let AssignmentValue::Comprehension(Comprehension::Array {
                output,
                iterator: _,
                filters: _,
            }) = value
            {
                // Check output: u.roles[0]
                if let Expr::IndexedAccess {
                    variable: var,
                    attribute: attr,
                    index,
                } = output.as_ref()
                {
                    assert_eq!(var, "u");
                    assert_eq!(attr, "roles");
                    assert!(matches!(index, Index::Number(0)));
                } else {
                    panic!("Expected indexed access in output");
                }
            } else {
                panic!("Expected array comprehension");
            }
        } else {
            panic!("Expected assignment");
        }
    }

    #[test]
    fn test_parse_comprehension_in_and_condition() {
        let input = r#"
            policy test {
                default: deny,
                rule complex_check {
                    allow if {
                        admin_names := {u.name | u := user.admins[_]; u.active == true} &&
                        user.name in admin_names
                    }
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        // The condition should be an AND
        if let Condition::And(conditions) = &policy.rules[0].condition {
            assert_eq!(conditions.len(), 2);

            // First condition: assignment with comprehension
            if let Condition::Assignment { variable, value } = &conditions[0] {
                assert_eq!(variable, "admin_names");
                assert!(matches!(
                    value,
                    AssignmentValue::Comprehension(Comprehension::Set { .. })
                ));
            } else {
                panic!("Expected assignment in first AND condition");
            }

            // Second condition: membership test
            if let Condition::Comparison { left, op, right } = &conditions[1] {
                if let ComparisonLeft::EntityAttr(attr) = left {
                    assert_eq!(attr.entity, Entity::User);
                    assert_eq!(attr.attribute, "name");
                } else {
                    panic!("Expected entity attribute in second condition");
                }
                assert_eq!(*op, Operator::In);
                if let ComparisonRight::Variable(var) = right {
                    assert_eq!(var, "admin_names");
                } else {
                    panic!("Expected variable reference");
                }
            } else {
                panic!("Expected comparison in second AND condition");
            }
        } else {
            panic!("Expected AND condition");
        }
    }

    // ===== Built-in Function Tests =====

    #[test]
    fn test_parse_method_call_count() {
        let input = r#"
            policy test {
                default: deny,
                rule count_check {
                    allow if perm_count := [perms.count() | perms := user.permissions[_]]
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Assignment { variable, value: _ } = &policy.rules[0].condition {
            assert_eq!(variable, "perm_count");
            // Method call in comprehension output - valid syntax
        } else {
            panic!("Expected assignment");
        }
    }

    #[test]
    fn test_parse_method_call_in_comprehension_output() {
        let input = r#"
            policy test {
                default: deny,
                rule lower_names {
                    allow if names := [u.name.lower() | u := user.users[_]]
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Assignment { variable, value } = &policy.rules[0].condition {
            assert_eq!(variable, "names");
            if let AssignmentValue::Comprehension(Comprehension::Array {
                output,
                iterator: _,
                filters: _,
            }) = value
            {
                // Output should be a method call: u.name.lower()
                if let Expr::MethodCall {
                    receiver,
                    method,
                    args,
                } = output.as_ref()
                {
                    // Receiver should be u.name (attribute access)
                    if let Expr::AttributeAccess {
                        variable,
                        attribute,
                    } = receiver.as_ref()
                    {
                        assert_eq!(variable, "u");
                        assert_eq!(attribute, "name");
                    } else {
                        panic!("Expected attribute access as receiver");
                    }
                    assert_eq!(*method, MethodName::Lower);
                    assert_eq!(args.len(), 0);
                } else {
                    panic!("Expected method call in output");
                }
            } else {
                panic!("Expected array comprehension");
            }
        } else {
            panic!("Expected assignment");
        }
    }

    #[test]
    fn test_parse_method_call_sum() {
        let input = r#"
            policy test {
                default: deny,
                rule sum_test {
                    allow if total := [u.score.sum() | u := user.users[_]]
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Assignment { variable, value } = &policy.rules[0].condition {
            assert_eq!(variable, "total");
            if let AssignmentValue::Comprehension(Comprehension::Array {
                output,
                iterator: _,
                filters: _,
            }) = value
            {
                if let Expr::MethodCall { method, .. } = output.as_ref() {
                    assert_eq!(*method, MethodName::Sum);
                } else {
                    panic!("Expected method call");
                }
            } else {
                panic!("Expected array comprehension");
            }
        } else {
            panic!("Expected assignment");
        }
    }

    #[test]
    fn test_parse_method_call_with_args() {
        let input = r#"
            policy test {
                default: deny,
                rule split_test {
                    allow if parts := [u.email.split("@") | u := user.users[_]]
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Assignment { variable, value } = &policy.rules[0].condition {
            assert_eq!(variable, "parts");
            if let AssignmentValue::Comprehension(Comprehension::Array {
                output,
                iterator: _,
                filters: _,
            }) = value
            {
                if let Expr::MethodCall {
                    receiver,
                    method,
                    args,
                } = output.as_ref()
                {
                    if let Expr::AttributeAccess {
                        variable,
                        attribute,
                    } = receiver.as_ref()
                    {
                        assert_eq!(variable, "u");
                        assert_eq!(attribute, "email");
                    }
                    assert_eq!(*method, MethodName::Split);
                    assert_eq!(args.len(), 1);
                    if let Expr::Literal(Value::String(s)) = &args[0] {
                        assert_eq!(s, "@");
                    } else {
                        panic!("Expected string argument");
                    }
                } else {
                    panic!("Expected method call");
                }
            } else {
                panic!("Expected array comprehension");
            }
        } else {
            panic!("Expected assignment");
        }
    }

    #[test]
    fn test_parse_function_call_is_string() {
        let input = r#"
            policy test {
                default: deny,
                rule type_check {
                    allow if strings := [u.name | u := user.users[_]; is_string(u.name)]
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Assignment { variable, value } = &policy.rules[0].condition {
            assert_eq!(variable, "strings");
            if let AssignmentValue::Comprehension(Comprehension::Array {
                output: _,
                iterator: _,
                filters,
            }) = value
            {
                assert_eq!(filters.len(), 1);
                // The filter should parse but we're testing the function call syntax here
                // Since filters are Condition not Expr, function calls in conditions need different handling
                // For now, let's test function calls in comprehension output
            } else {
                panic!("Expected array comprehension");
            }
        } else {
            panic!("Expected assignment");
        }
    }

    #[test]
    fn test_parse_method_call_max() {
        let input = r#"
            policy test {
                default: deny,
                rule max_test {
                    allow if max_val := [scores.max() | scores := user.scores[_]]
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Assignment { variable, value } = &policy.rules[0].condition {
            assert_eq!(variable, "max_val");
            if let AssignmentValue::Comprehension(Comprehension::Array {
                output,
                iterator: _,
                filters: _,
            }) = value
            {
                if let Expr::MethodCall { method, .. } = output.as_ref() {
                    assert_eq!(*method, MethodName::Max);
                } else {
                    panic!("Expected method call");
                }
            } else {
                panic!("Expected array comprehension");
            }
        } else {
            panic!("Expected assignment");
        }
    }

    #[test]
    fn test_parse_method_call_min() {
        let input = r#"
            policy test {
                default: deny,
                rule min_test {
                    allow if min_val := [scores.min() | scores := user.scores[_]]
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Assignment { variable, value } = &policy.rules[0].condition {
            assert_eq!(variable, "min_val");
            if let AssignmentValue::Comprehension(Comprehension::Array {
                output,
                iterator: _,
                filters: _,
            }) = value
            {
                if let Expr::MethodCall { method, .. } = output.as_ref() {
                    assert_eq!(*method, MethodName::Min);
                } else {
                    panic!("Expected method call");
                }
            } else {
                panic!("Expected array comprehension");
            }
        } else {
            panic!("Expected assignment");
        }
    }

    #[test]
    fn test_parse_method_call_upper() {
        let input = r#"
            policy test {
                default: deny,
                rule upper_test {
                    allow if codes := [u.code.upper() | u := user.users[_]]
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Assignment { variable, value } = &policy.rules[0].condition {
            assert_eq!(variable, "codes");
            if let AssignmentValue::Comprehension(Comprehension::Array {
                output,
                iterator: _,
                filters: _,
            }) = value
            {
                if let Expr::MethodCall { method, .. } = output.as_ref() {
                    assert_eq!(*method, MethodName::Upper);
                } else {
                    panic!("Expected method call");
                }
            } else {
                panic!("Expected array comprehension");
            }
        } else {
            panic!("Expected assignment");
        }
    }

    #[test]
    fn test_parse_method_call_trim() {
        let input = r#"
            policy test {
                default: deny,
                rule trim_test {
                    allow if names := [u.name.trim() | u := user.users[_]]
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Assignment { variable, value } = &policy.rules[0].condition {
            assert_eq!(variable, "names");
            if let AssignmentValue::Comprehension(Comprehension::Array {
                output,
                iterator: _,
                filters: _,
            }) = value
            {
                if let Expr::MethodCall { method, .. } = output.as_ref() {
                    assert_eq!(*method, MethodName::Trim);
                } else {
                    panic!("Expected method call");
                }
            } else {
                panic!("Expected array comprehension");
            }
        } else {
            panic!("Expected assignment");
        }
    }

    #[test]
    fn test_parse_method_call_contains() {
        let input = r#"
            policy test {
                default: deny,
                rule contains_test {
                    allow if matches := [u.role.contains("admin") | u := user.users[_]]
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Assignment { variable, value: _ } = &policy.rules[0].condition {
            assert_eq!(variable, "matches");
            // Test parses successfully with contains() in output expression
        } else {
            panic!("Expected assignment");
        }
    }

    #[test]
    fn test_parse_method_call_startswith() {
        let input = r#"
            policy test {
                default: deny,
                rule prefix_test {
                    allow if starts := [u.email.startswith("admin") | u := user.users[_]]
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        assert_eq!(policy.rules[0].name, "prefix_test");
    }

    #[test]
    fn test_parse_method_call_endswith() {
        let input = r#"
            policy test {
                default: deny,
                rule suffix_test {
                    allow if ends := [u.email.endswith("@company.com") | u := user.users[_]]
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        assert_eq!(policy.rules[0].name, "suffix_test");
    }

    #[test]
    fn test_parse_method_call_union() {
        let input = r#"
            policy test {
                default: deny,
                rule union_test {
                    allow if all_perms := [user_perms.union(role_perms) | user_perms := user.perms[_]]
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Assignment { variable, value } = &policy.rules[0].condition {
            assert_eq!(variable, "all_perms");
            if let AssignmentValue::Comprehension(Comprehension::Array {
                output,
                iterator: _,
                filters: _,
            }) = value
            {
                if let Expr::MethodCall { method, args, .. } = output.as_ref() {
                    assert_eq!(*method, MethodName::Union);
                    assert_eq!(args.len(), 1);
                } else {
                    panic!("Expected method call");
                }
            } else {
                panic!("Expected array comprehension");
            }
        } else {
            panic!("Expected assignment");
        }
    }

    #[test]
    fn test_parse_method_call_intersection() {
        let input = r#"
            policy test {
                default: deny,
                rule intersection_test {
                    allow if common := [a.intersection(b) | a := user.sets[_]]
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Assignment { variable, value } = &policy.rules[0].condition {
            assert_eq!(variable, "common");
            if let AssignmentValue::Comprehension(Comprehension::Array {
                output,
                iterator: _,
                filters: _,
            }) = value
            {
                if let Expr::MethodCall { method, .. } = output.as_ref() {
                    assert_eq!(*method, MethodName::Intersection);
                } else {
                    panic!("Expected method call");
                }
            } else {
                panic!("Expected array comprehension");
            }
        } else {
            panic!("Expected assignment");
        }
    }

    #[test]
    fn test_parse_method_call_difference() {
        let input = r#"
            policy test {
                default: deny,
                rule difference_test {
                    allow if diff := [a.difference(b) | a := user.sets[_]]
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Assignment { variable, value } = &policy.rules[0].condition {
            assert_eq!(variable, "diff");
            if let AssignmentValue::Comprehension(Comprehension::Array {
                output,
                iterator: _,
                filters: _,
            }) = value
            {
                if let Expr::MethodCall { method, .. } = output.as_ref() {
                    assert_eq!(*method, MethodName::Difference);
                } else {
                    panic!("Expected method call");
                }
            } else {
                panic!("Expected array comprehension");
            }
        } else {
            panic!("Expected assignment");
        }
    }

    #[test]
    fn test_parse_function_call_concat() {
        let input = r#"
            policy test {
                default: deny,
                rule concat_test {
                    allow if full_names := [concat(u.first, " ", u.last) | u := user.users[_]]
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Assignment { variable, value } = &policy.rules[0].condition {
            assert_eq!(variable, "full_names");
            if let AssignmentValue::Comprehension(Comprehension::Array {
                output,
                iterator: _,
                filters: _,
            }) = value
            {
                if let Expr::FunctionCall {
                    namespace,
                    function,
                    args,
                } = output.as_ref()
                {
                    assert_eq!(namespace, &None);
                    assert_eq!(function, "concat");
                    assert_eq!(args.len(), 3);
                } else {
                    panic!("Expected function call");
                }
            } else {
                panic!("Expected array comprehension");
            }
        } else {
            panic!("Expected assignment");
        }
    }

    #[test]
    fn test_parse_method_call_chaining() {
        let input = r#"
            policy test {
                default: deny,
                rule chain_test {
                    allow if clean_names := [u.name.trim().lower() | u := user.users[_]]
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Assignment { variable, value } = &policy.rules[0].condition {
            assert_eq!(variable, "clean_names");
            if let AssignmentValue::Comprehension(Comprehension::Array {
                output,
                iterator: _,
                filters: _,
            }) = value
            {
                // Output should be a chained method call: u.name.trim().lower()
                if let Expr::MethodCall {
                    receiver,
                    method,
                    args: _,
                } = output.as_ref()
                {
                    // Outer call is .lower()
                    assert_eq!(*method, MethodName::Lower);
                    // Receiver should be u.name.trim() (another method call)
                    if let Expr::MethodCall {
                        receiver: inner_receiver,
                        method: inner_method,
                        args: _,
                    } = receiver.as_ref()
                    {
                        assert_eq!(*inner_method, MethodName::Trim);
                        // Inner receiver should be u.name
                        if let Expr::AttributeAccess {
                            variable,
                            attribute,
                        } = inner_receiver.as_ref()
                        {
                            assert_eq!(variable, "u");
                            assert_eq!(attribute, "name");
                        } else {
                            panic!("Expected attribute access in inner receiver");
                        }
                    } else {
                        panic!("Expected method call as receiver for chaining");
                    }
                } else {
                    panic!("Expected method call in output");
                }
            } else {
                panic!("Expected array comprehension");
            }
        } else {
            panic!("Expected assignment");
        }
    }
}

#[test]
fn test_parse_time_now_ns() {
    let input = r#"
            policy test {
                default: deny,
                rule time_check {
                    allow if now := time::now_ns()
                }
            }
        "#;

    let policy = ReapParser::parse(input).unwrap();
    assert_eq!(policy.rules.len(), 1);
    // Verify it parses correctly
    if let Condition::Assignment { variable, value } = &policy.rules[0].condition {
        assert_eq!(variable, "now");
        if let AssignmentValue::Variable(ref _v) = value {
            // This would actually be a function call expression
            // Just verify it compiles
        }
    }
}

#[test]
fn test_parse_time_parse_rfc3339() {
    let input = r#"
            policy test {
                default: deny,
                rule time_check {
                    allow if timestamp := time::parse_rfc3339("2025-01-01T00:00:00Z")
                }
            }
        "#;

    let policy = ReapParser::parse(input).unwrap();
    assert_eq!(policy.rules.len(), 1);
}

#[test]
fn test_parse_time_arithmetic() {
    let input = r#"
            policy test {
                default: deny,
                rule time_check {
                    allow if future := time::add_ns(time::now_ns(), 3600000000000)
                }
            }
        "#;

    let policy = ReapParser::parse(input).unwrap();
    assert_eq!(policy.rules.len(), 1);
}

#[test]
fn test_parse_time_comparison() {
    let input = r#"
            policy test {
                default: deny,
                rule time_check {
                    allow if time::is_before(user.expires_at, time::now_ns())
                }
            }
        "#;

    let policy = ReapParser::parse(input).unwrap();
    assert_eq!(policy.rules.len(), 1);
}

#[test]
fn test_parse_regex_matches() {
    let input = r#"
            policy test {
                default: deny,
                rule email_validation {
                    allow if valid_emails := [e.matches("^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\\.[a-zA-Z]{2,}$") | e := user.emails[_]]
                }
            }
        "#;

    let policy = ReapParser::parse(input).unwrap();
    assert_eq!(policy.rules.len(), 1);
}

#[test]
fn test_parse_regex_find() {
    let input = r#"
            policy test {
                default: deny,
                rule pattern_extract {
                    allow if matches := [t.find("\\d+") | t := user.texts[_]]
                }
            }
        "#;

    let policy = ReapParser::parse(input).unwrap();
    assert_eq!(policy.rules.len(), 1);
}

#[test]
fn test_parse_regex_replace() {
    let input = r#"
            policy test {
                default: deny,
                rule sanitize {
                    allow if clean_values := [inp.replace("[^a-zA-Z0-9]", "") | inp := user.inputs[_]]
                }
            }
        "#;

    let policy = ReapParser::parse(input).unwrap();
    assert_eq!(policy.rules.len(), 1);
}

#[test]
fn test_parse_regex_namespace_functions() {
    let input = r#"
            policy test {
                default: deny,
                rule validate_pattern {
                    allow if pattern := "[a-z]+"
                    && regex::is_valid(pattern)
                    && escaped := regex::escape(user.input)
                }
            }
        "#;

    let policy = ReapParser::parse(input).unwrap();
    assert_eq!(policy.rules.len(), 1);
}

#[test]
fn test_parse_math_abs_and_round() {
    let input = r#"
            policy test {
                default: deny,
                rule math_rounding {
                    allow if abs_val := math::abs(-42)
                    && rounded := math::round(3.7)
                }
            }
        "#;

    let policy = ReapParser::parse(input).unwrap();
    assert_eq!(policy.rules.len(), 1);
}

#[test]
fn test_parse_math_floor_ceil() {
    let input = r#"
            policy test {
                default: deny,
                rule math_floor_ceil {
                    allow if floor_val := math::floor(3.9)
                    && ceil_val := math::ceil(3.1)
                }
            }
        "#;

    let policy = ReapParser::parse(input).unwrap();
    assert_eq!(policy.rules.len(), 1);
}

#[test]
fn test_parse_math_pow_sqrt() {
    let input = r#"
            policy test {
                default: deny,
                rule math_power {
                    allow if squared := math::pow(5, 2)
                    && sqrt_val := math::sqrt(16)
                }
            }
        "#;

    let policy = ReapParser::parse(input).unwrap();
    assert_eq!(policy.rules.len(), 1);
}

#[test]
fn test_parse_math_min_max_clamp() {
    let input = r#"
            policy test {
                default: deny,
                rule math_comparisons {
                    allow if min_val := math::min(10, 20)
                    && max_val := math::max(10, 20)
                    && clamped := math::clamp(150, 0, 100)
                }
            }
        "#;

    let policy = ReapParser::parse(input).unwrap();
    assert_eq!(policy.rules.len(), 1);
}

#[test]
fn test_parse_collection_first_last() {
    let input = r#"
            policy test {
                default: deny,
                rule array_access {
                    allow if first_names := [arr.first() | arr := user.lists[_]]
                    && last_items := [a.last() | a := resource.arrays[_]]
                }
            }
        "#;

    let policy = ReapParser::parse(input).unwrap();
    assert_eq!(policy.rules.len(), 1);
}

#[test]
fn test_parse_collection_slice_reverse() {
    let input = r#"
            policy test {
                default: deny,
                rule array_manipulation {
                    allow if sliced_arrays := [arr.slice(1, 4) | arr := user.data[_]]
                    && reversed_lists := [lst.reverse() | lst := resource.lists[_]]
                }
            }
        "#;

    let policy = ReapParser::parse(input).unwrap();
    assert_eq!(policy.rules.len(), 1);
}

#[test]
fn test_parse_collection_sort_unique() {
    let input = r#"
            policy test {
                default: deny,
                rule array_processing {
                    allow if sorted_nums := [nums.sort() | nums := user.numbers[_]]
                    && unique_vals := [vals.unique() | vals := resource.values[_]]
                }
            }
        "#;

    let policy = ReapParser::parse(input).unwrap();
    assert_eq!(policy.rules.len(), 1);
}

#[test]
fn test_parse_object_methods() {
    let input = r#"
            policy test {
                default: deny,
                rule object_access {
                    allow if all_keys := [obj.keys() | obj := user.objects[_]]
                    && all_values := [o.values() | o := resource.data[_]]
                    && has_role := [obj.has_key("role") | obj := context.metadata[_]]
                }
            }
        "#;

    let policy = ReapParser::parse(input).unwrap();
    assert_eq!(policy.rules.len(), 1);
}
