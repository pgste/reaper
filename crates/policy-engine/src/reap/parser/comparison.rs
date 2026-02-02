//! Comparison and assignment parsing for .reap files.
//!
//! This module handles parsing of:
//! - Variable assignments: x := expr
//! - Comparisons: left op right
//! - Membership tests: value in collection

use super::comprehension::parse_comprehension;
use super::expression::{
    parse_comp_function_call, parse_entity_method_call, parse_var_method_call,
};
use super::value::{parse_bracket_index, parse_entity_attr, parse_value, parse_var_attr};
use super::Rule;
use crate::reap::ast::*;
use reaper_core::ReaperError;

/// Parse assignment expression: x := value
pub(super) fn parse_assignment(pair: pest::iterators::Pair<Rule>) -> Result<Condition, ReaperError> {
    let mut inner = pair.into_inner();

    let variable = inner.next().unwrap().as_str().to_string();
    let value_pair = inner.next().unwrap();

    let value = parse_assignment_value(value_pair)?;

    Ok(Condition::Assignment { variable, value })
}

/// Parse assignment value (right side of :=)
pub(super) fn parse_assignment_value(
    pair: pest::iterators::Pair<Rule>,
) -> Result<AssignmentValue, ReaperError> {
    // The pair is assignment_value, which contains comprehension, comp_function_call, comparison, var_method_call, entity_attr, ident[index], value, or ident

    // Collect all inner pairs to handle (ident ~ bracket_index) pattern
    let mut inner_pairs: Vec<_> = pair.into_inner().collect();

    // Check if this is an indexed variable access: ident ~ bracket_index
    if inner_pairs.len() == 2
        && inner_pairs[0].as_rule() == Rule::ident
        && inner_pairs[1].as_rule() == Rule::bracket_index
    {
        let var_name = inner_pairs[0].as_str().to_string();
        let index_pair = inner_pairs.remove(1).into_inner().next().unwrap();
        let index = parse_bracket_index(index_pair)?;

        // Create an Expr::IndexedAccess
        // Since there's no attribute access (just direct variable index), use empty attribute
        return Ok(AssignmentValue::Expr(Expr::IndexedAccess {
            variable: var_name,
            attribute: String::new(), // No attribute, just direct variable access
            index,
        }));
    }

    let inner = inner_pairs
        .into_iter()
        .next()
        .ok_or_else(|| ReaperError::InvalidPolicy {
            reason: "Empty assignment value".to_string(),
        })?;

    match inner.as_rule() {
        Rule::comprehension => Ok(AssignmentValue::Comprehension(parse_comprehension(inner)?)),
        Rule::comp_function_call => Ok(AssignmentValue::Expr(parse_comp_function_call(inner)?)),
        Rule::entity_method_call => Ok(AssignmentValue::Expr(parse_entity_method_call(inner)?)),
        Rule::var_method_call => Ok(AssignmentValue::Expr(parse_var_method_call(inner)?)),
        Rule::comparison => {
            // Parse comparison and extract the comparison fields
            let mut inner_iter = inner.into_inner();
            let first_pair = inner_iter.next().unwrap();

            // Check if this is "value in entity_attr" or "value in var_attr" form
            if first_pair.as_rule() == Rule::value {
                // Format: value in attr - not yet supported in assignment
                return Err(ReaperError::InvalidPolicy {
                    reason: "'in' comparisons cannot be assigned to variables yet".to_string(),
                });
            }

            // Standard comparison: left op right
            let left = match first_pair.as_rule() {
                Rule::entity_method_call => {
                    ComparisonLeft::Expr(parse_entity_method_call(first_pair)?)
                }
                Rule::entity_attr => ComparisonLeft::EntityAttr(parse_entity_attr(first_pair)?),
                Rule::var_method_call => ComparisonLeft::Expr(parse_var_method_call(first_pair)?),
                Rule::var_attr => ComparisonLeft::VarAttr(parse_var_attr(first_pair)?),
                Rule::ident => {
                    ComparisonLeft::Expr(Expr::Variable(first_pair.as_str().to_string()))
                }
                _ => {
                    return Err(ReaperError::InvalidPolicy {
                        reason: format!(
                            "Unexpected left side of comparison: {:?}",
                            first_pair.as_rule()
                        ),
                    })
                }
            };

            let op_pair = inner_iter.next().unwrap();
            let op = Operator::from(op_pair.as_str());

            let right_pair = inner_iter.next().unwrap();
            let right = match right_pair.as_rule() {
                Rule::comparison_right => {
                    // Unwrap the comparison_right rule
                    parse_comparison_right(right_pair)?
                }
                Rule::entity_attr => ComparisonRight::EntityAttr(parse_entity_attr(right_pair)?),
                Rule::var_attr => ComparisonRight::VarAttr(parse_var_attr(right_pair)?),
                Rule::value => ComparisonRight::Value(parse_value(right_pair)?),
                Rule::ident => ComparisonRight::Variable(right_pair.as_str().to_string()),
                _ => {
                    return Err(ReaperError::InvalidPolicy {
                        reason: format!(
                            "Unexpected right side of comparison: {:?}",
                            right_pair.as_rule()
                        ),
                    })
                }
            };

            Ok(AssignmentValue::Comparison { left, op, right })
        }
        Rule::entity_attr => Ok(AssignmentValue::EntityAttr(parse_entity_attr(inner)?)),
        Rule::var_attr => {
            // Var attr needs to be converted to an Expr
            let var_attr = parse_var_attr(inner)?;
            // If there's an index, use IndexedAccess
            if let Some(index) = var_attr.index {
                Ok(AssignmentValue::Expr(Expr::IndexedAccess {
                    variable: var_attr.variable,
                    attribute: var_attr.attribute,
                    index,
                }))
            } else {
                Ok(AssignmentValue::Expr(Expr::AttributeAccess {
                    variable: var_attr.variable,
                    attribute: var_attr.attribute,
                }))
            }
        }
        Rule::value => Ok(AssignmentValue::Value(parse_value(inner)?)),
        Rule::ident => Ok(AssignmentValue::Variable(inner.as_str().to_string())),
        _ => Err(ReaperError::InvalidPolicy {
            reason: format!("Unexpected assignment value type: {:?}", inner.as_rule()),
        }),
    }
}

/// Parse comparison expression: left op right
pub(super) fn parse_comparison(pair: pest::iterators::Pair<Rule>) -> Result<Condition, ReaperError> {
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
            Rule::ident => ComparisonLeft::Expr(Expr::Variable(next_pair.as_str().to_string())),
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

    // Parse the left side (entity_method_call, entity_attr, var_attr, var_method_call, or simple ident)
    let left = match first_pair.as_rule() {
        Rule::entity_method_call => ComparisonLeft::Expr(parse_entity_method_call(first_pair)?),
        Rule::entity_attr => ComparisonLeft::EntityAttr(parse_entity_attr(first_pair)?),
        Rule::var_attr => ComparisonLeft::VarAttr(parse_var_attr(first_pair)?),
        Rule::var_method_call => ComparisonLeft::Expr(parse_var_method_call(first_pair)?),
        Rule::ident => {
            // Simple variable on left side - wrap as Variable expression
            ComparisonLeft::Expr(Expr::Variable(first_pair.as_str().to_string()))
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

/// Parse right side of comparison
pub(super) fn parse_comparison_right(
    pair: pest::iterators::Pair<Rule>,
) -> Result<ComparisonRight, ReaperError> {
    // The pair is comparison_right, which contains var_method_call, entity_attr, var_attr, value, or ident
    let inner = pair
        .into_inner()
        .next()
        .ok_or_else(|| ReaperError::InvalidPolicy {
            reason: "Empty comparison right".to_string(),
        })?;

    match inner.as_rule() {
        Rule::var_method_call => Ok(ComparisonRight::Expr(parse_var_method_call(inner)?)),
        Rule::entity_attr => Ok(ComparisonRight::EntityAttr(parse_entity_attr(inner)?)),
        Rule::var_attr => Ok(ComparisonRight::VarAttr(parse_var_attr(inner)?)),
        Rule::value => Ok(ComparisonRight::Value(parse_value(inner)?)),
        Rule::ident => Ok(ComparisonRight::Variable(inner.as_str().to_string())),
        _ => Err(ReaperError::InvalidPolicy {
            reason: format!("Unexpected comparison right type: {:?}", inner.as_rule()),
        }),
    }
}
