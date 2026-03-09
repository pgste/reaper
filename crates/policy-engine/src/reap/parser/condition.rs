//! Condition parsing for .reap files.
//!
//! This module handles parsing of condition expressions:
//! - Boolean logic: AND, OR, NOT
//! - Primary expressions: comparisons, assignments, function calls

use super::comparison::{parse_assignment, parse_comparison};
use super::expression::{
    parse_comp_function_call, parse_entity_method_call, parse_var_method_call,
};
use super::Rule;
use crate::reap::ast::*;
use reaper_core::ReaperError;

/// Parse a condition block
pub(super) fn parse_condition(pair: pest::iterators::Pair<Rule>) -> Result<Condition, ReaperError> {
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

/// Parse condition expression (entry point for boolean expressions)
pub(super) fn parse_condition_expr(
    pair: pest::iterators::Pair<Rule>,
) -> Result<Condition, ReaperError> {
    let inner = pair.into_inner().next().unwrap();
    parse_or_expr(inner)
}

/// Parse OR expression: expr || expr || ...
pub(super) fn parse_or_expr(pair: pest::iterators::Pair<Rule>) -> Result<Condition, ReaperError> {
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

/// Parse AND expression: expr && expr && ...
pub(super) fn parse_and_expr(pair: pest::iterators::Pair<Rule>) -> Result<Condition, ReaperError> {
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

/// Parse NOT expression: !expr
pub(super) fn parse_not_expr(pair: pest::iterators::Pair<Rule>) -> Result<Condition, ReaperError> {
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

/// Parse primary expression (leaf of boolean expression tree)
pub(super) fn parse_primary_expr(
    pair: pest::iterators::Pair<Rule>,
) -> Result<Condition, ReaperError> {
    let inner = pair.into_inner().next().unwrap();

    match inner.as_rule() {
        Rule::condition_expr => parse_condition_expr(inner),
        Rule::assignment => parse_assignment(inner),
        Rule::comparison => parse_comparison(inner),
        Rule::entity_method_call => {
            // Parse entity method call and wrap it as an expression condition
            let expr = parse_entity_method_call(inner)?;
            Ok(Condition::Expr(expr))
        }
        Rule::var_method_call => {
            // Parse variable method call and wrap it as an expression condition
            let expr = parse_var_method_call(inner)?;
            Ok(Condition::Expr(expr))
        }
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
        Rule::ident => {
            // Variable reference in condition (for boolean variables)
            Ok(Condition::Expr(Expr::Variable(inner.as_str().to_string())))
        }
        _ => Err(ReaperError::InvalidPolicy {
            reason: format!("Unexpected rule in primary_expr: {:?}", inner.as_rule()),
        }),
    }
}
