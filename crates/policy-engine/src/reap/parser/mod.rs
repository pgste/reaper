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
        // Bound syntactic nesting BEFORE handing the input to pest: pest parses
        // by recursive descent, so deeply nested `(...)`/`!...` would overflow
        // the stack during parsing itself. This lexical pre-scan rejects such
        // input first, keeping the DSL total/terminating (Plan 05, Step 2).
        crate::reap::limits::enforce_source_nesting(input)?;

        let pairs = <Self as PestParser<Rule>>::parse(Rule::policy, input).map_err(|e| {
            ReaperError::InvalidPolicy {
                reason: format!("Parse error: {}", e),
            }
        })?;

        let mut policy_name = String::new();
        let mut metadata = HashMap::new();
        let mut default_decision = None;
        let mut rules = Vec::new();
        let mut functions = Vec::new();
        let mut imports = Vec::new();

        for pair in pairs {
            if pair.as_rule() == Rule::policy {
                for inner_pair in pair.into_inner() {
                    match inner_pair.as_rule() {
                        Rule::import_stmt => {
                            imports.push(parse_import_stmt(inner_pair)?);
                        }
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
                                    Rule::func_def => {
                                        functions.push(parse_func_def(item)?);
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

        let policy = Policy {
            name: policy_name,
            metadata,
            default_decision,
            rules,
            functions,
            imports,
        };

        // Belt-and-suspenders: the pre-scan bounds the source, but re-check the
        // built AST so the depth guarantee holds even if the lexical accounting
        // and the true tree shape ever diverge. (Func-aware: validates the
        // function set — names, call-graph DAG — and depth-accounts call
        // expansions.)
        crate::reap::limits::enforce_policy_depth(&policy)?;

        Ok(policy)
    }

    /// Parse a `.reap` LIBRARY file: `library name { func ... }`. Returns the
    /// library name and its function definitions (namespace `None` — the
    /// importer namespaces them under the import alias).
    pub fn parse_library(input: &str) -> Result<(String, Vec<FuncDef>), ReaperError> {
        crate::reap::limits::enforce_source_nesting(input)?;

        let pairs = <Self as PestParser<Rule>>::parse(Rule::library, input).map_err(|e| {
            ReaperError::InvalidPolicy {
                reason: format!("Library parse error: {}", e),
            }
        })?;

        let mut name = String::new();
        let mut functions = Vec::new();
        for pair in pairs {
            if pair.as_rule() == Rule::library {
                for inner_pair in pair.into_inner() {
                    match inner_pair.as_rule() {
                        Rule::ident => name = inner_pair.as_str().to_string(),
                        Rule::func_def => functions.push(parse_func_def(inner_pair)?),
                        // metadata fields are allowed but carry no semantics yet
                        _ => {}
                    }
                }
            }
        }
        Ok((name, functions))
    }
}

/// Parse an import statement: import "path" as alias
fn parse_import_stmt(pair: pest::iterators::Pair<Rule>) -> Result<ImportDecl, ReaperError> {
    let mut inner = pair.into_inner();
    let path_pair = inner.next().unwrap();
    let path = parse_string_literal(path_pair)?;
    let alias = inner.next().unwrap().as_str().to_string();
    Ok(ImportDecl { path, alias })
}

/// Parse a helper predicate: func name(params) := condition
fn parse_func_def(pair: pest::iterators::Pair<Rule>) -> Result<FuncDef, ReaperError> {
    let mut inner = pair.into_inner();
    let name = inner.next().unwrap().as_str().to_string();

    let mut next = inner.next().unwrap();
    let params = if next.as_rule() == Rule::func_params {
        let params: Vec<String> = next
            .clone()
            .into_inner()
            .map(|p| p.as_str().to_string())
            .collect();
        next = inner.next().unwrap();
        params
    } else {
        Vec::new()
    };

    let body = parse_condition(next)?;
    Ok(FuncDef {
        name,
        namespace: None,
        params,
        body,
    })
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

    let mut next = inner.next().unwrap();
    let message = if next.as_rule() == Rule::message_clause {
        let expr_pair = next.into_inner().next().unwrap();
        let expr = super::parser::expression::parse_comp_expr(expr_pair)?;
        next = inner.next().unwrap();
        Some(expr)
    } else {
        None
    };
    let condition = parse_condition(next)?;

    Ok(crate::reap::ast::Rule {
        message,
        name,
        decision,
        condition,
    })
}

#[cfg(test)]
mod tests;
