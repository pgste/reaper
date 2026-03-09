//! Parser for .reap files using Pest
//!
//! This module provides parsing of Reaper policy DSL files into an AST.
//!
//! ## Module Structure
//!
//! - `condition`: Boolean condition parsing (AND, OR, NOT)
//! - `comparison`: Comparison and assignment parsing
//! - `comprehension`: Comprehension parsing (set, array, object)
//! - `expression`: Expression and method call parsing
//! - `value`: Value and attribute parsing

mod comparison;
mod comprehension;
mod condition;
mod expression;
mod value;

use crate::reap::ast::*;
use pest::Parser as PestParser;
use pest_derive::Parser;
use reaper_core::ReaperError;
use std::collections::HashMap;

// Re-export for internal use
use condition::parse_condition;
use value::parse_string_literal;

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

/// Parse metadata field: key = "value"
fn parse_metadata_field(
    pair: pest::iterators::Pair<Rule>,
) -> Result<(String, String), ReaperError> {
    let mut inner = pair.into_inner();
    let key = inner.next().unwrap().as_str().to_string();
    let value_pair = inner.next().unwrap();
    let value = parse_string_literal(value_pair)?;
    Ok((key, value))
}

/// Parse default field: default = allow|deny
fn parse_default_field(pair: pest::iterators::Pair<Rule>) -> Result<Decision, ReaperError> {
    let mut inner = pair.into_inner();
    let decision_pair = inner.next().unwrap();
    Ok(Decision::from(decision_pair.as_str()))
}

/// Parse a rule: rule name = decision when condition
fn parse_rule(pair: pest::iterators::Pair<Rule>) -> Result<crate::reap::ast::Rule, ReaperError> {
    let mut inner = pair.into_inner();

    let name = inner.next().unwrap().as_str().to_string();
    let decision_pair = inner.next().unwrap();
    let decision = Decision::from(decision_pair.as_str());
    let condition_pair = inner.next().unwrap();
    let condition = parse_condition(condition_pair)?;

    Ok(crate::reap::ast::Rule {
        name,
        decision,
        condition,
    })
}

#[cfg(test)]
mod tests;
