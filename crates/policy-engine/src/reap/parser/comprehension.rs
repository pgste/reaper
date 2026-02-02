//! Comprehension parsing for .reap files.
//!
//! This module handles parsing of comprehensions:
//! - Set comprehensions: {expr | iter; filters}
//! - Array comprehensions: [expr | iter; filters]
//! - Object comprehensions: {key: value | iter; filters}

use super::condition::parse_condition;
use super::expression::parse_comp_expr;
use super::value::{parse_bracket_index, parse_entity_attr, parse_var_attr};
use super::Rule;
use crate::reap::ast::*;
use reaper_core::ReaperError;

/// Parse a comprehension expression
/// Comprehensions collect and transform data from collections
pub(super) fn parse_comprehension(
    pair: pest::iterators::Pair<Rule>,
) -> Result<Comprehension, ReaperError> {
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
pub(super) fn parse_set_comprehension(
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
pub(super) fn parse_array_comprehension(
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
pub(super) fn parse_object_comprehension(
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
pub(super) fn parse_comp_iterator(
    pair: pest::iterators::Pair<Rule>,
) -> Result<ComprehensionIterator, ReaperError> {
    let mut variable: Option<String> = None;
    let mut collection: Option<IterationSource> = None;
    let mut last_ident: Option<String> = None;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::ident => {
                if variable.is_none() {
                    variable = Some(inner.as_str().to_string());
                } else {
                    // This is the second ident, part of "ident ~ bracket_index"
                    last_ident = Some(inner.as_str().to_string());
                }
            }
            Rule::entity_attr => {
                collection = Some(IterationSource::EntityAttr(parse_entity_attr(inner)?));
            }
            Rule::var_attr => {
                collection = Some(IterationSource::VarAttr(parse_var_attr(inner)?));
            }
            Rule::bracket_index => {
                // This is part of "ident ~ bracket_index", build an IndexedVariable
                if let Some(ident_name) = last_ident.take() {
                    let index_pair = inner.into_inner().next().unwrap();
                    let index = parse_bracket_index(index_pair)?;
                    collection = Some(IterationSource::IndexedVariable {
                        variable: ident_name,
                        index,
                    });
                }
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
pub(super) fn parse_comp_filters(
    pair: pest::iterators::Pair<Rule>,
) -> Result<Vec<Condition>, ReaperError> {
    let mut filters = Vec::new();

    for inner in pair.into_inner() {
        if inner.as_rule() == Rule::condition {
            filters.push(parse_condition(inner)?);
        }
    }

    Ok(filters)
}
