//! Expression parsing for .reap files.
//!
//! This module handles parsing of:
//! - Comprehension output expressions
//! - Method calls (variable and entity)
//! - Function calls
//! - Method chains

use super::value::{parse_bracket_index, parse_value};
use super::Rule;
use crate::reap::ast::*;
use reaper_core::ReaperError;

/// Parse comprehension output expression
pub(super) fn parse_comp_expr(pair: pest::iterators::Pair<Rule>) -> Result<Expr, ReaperError> {
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
pub(super) fn parse_comp_base_expr(pair: pest::iterators::Pair<Rule>) -> Result<Expr, ReaperError> {
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
pub(super) fn parse_comp_dot_access(pair: pest::iterators::Pair<Rule>) -> Result<Expr, ReaperError> {
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
pub(super) fn parse_comp_method_chain(
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

/// Parse variable method call: skills.count(), tags.intersection([...]), d.permissions.contains("x")
pub(super) fn parse_var_method_call(
    pair: pest::iterators::Pair<Rule>,
) -> Result<Expr, ReaperError> {
    let mut idents: Vec<String> = Vec::new();
    let mut method_chain: Option<pest::iterators::Pair<Rule>> = None;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::ident => {
                idents.push(inner.as_str().to_string());
            }
            Rule::comp_method_chain => {
                method_chain = Some(inner);
            }
            _ => {}
        }
    }

    if idents.is_empty() {
        return Err(ReaperError::InvalidPolicy {
            reason: "Variable method call missing variable name".to_string(),
        });
    }

    let chain = method_chain.ok_or_else(|| ReaperError::InvalidPolicy {
        reason: "Variable method call missing method chain".to_string(),
    })?;

    // Start with the first identifier as the variable
    let mut receiver = Expr::Variable(idents[0].clone());

    // Build attribute access chain for remaining identifiers (if any)
    // e.g., d.permissions becomes Variable("d") -> AttributeAccess{variable: "d", attribute: "permissions"}
    for attr in idents.iter().skip(1) {
        // Get the variable name from the current receiver
        let var_name = match &receiver {
            Expr::Variable(name) => name.clone(),
            Expr::AttributeAccess { variable, .. } => variable.clone(),
            _ => {
                return Err(ReaperError::InvalidPolicy {
                    reason: "Invalid receiver for attribute access".to_string(),
                })
            }
        };

        receiver = Expr::AttributeAccess {
            variable: var_name,
            attribute: attr.clone(),
        };
    }

    // Apply the method chain to the receiver
    parse_comp_method_chain(receiver, chain)
}

/// Parse entity method call: user.name.lower(), resource.title.upper()
pub(super) fn parse_entity_method_call(
    pair: pest::iterators::Pair<Rule>,
) -> Result<Expr, ReaperError> {
    let mut entity: Option<Entity> = None;
    let mut idents: Vec<String> = Vec::new();
    let mut method_chain: Option<pest::iterators::Pair<Rule>> = None;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::entity => {
                entity = Some(Entity::from(inner.as_str()));
            }
            Rule::ident => {
                idents.push(inner.as_str().to_string());
            }
            Rule::comp_method_chain => {
                method_chain = Some(inner);
            }
            _ => {}
        }
    }

    let entity = entity.ok_or_else(|| ReaperError::InvalidPolicy {
        reason: "Entity method call missing entity".to_string(),
    })?;

    if idents.is_empty() {
        return Err(ReaperError::InvalidPolicy {
            reason: "Entity method call missing attribute".to_string(),
        });
    }

    let chain = method_chain.ok_or_else(|| ReaperError::InvalidPolicy {
        reason: "Entity method call missing method chain".to_string(),
    })?;

    // Build a pseudo-variable path: "user.name"
    let entity_path = format!("{:?}.{}", entity, idents[0]).to_lowercase();
    let mut receiver = Expr::Variable(entity_path);

    // Build attribute access chain for remaining identifiers before methods (if any)
    for attr in idents.iter().skip(1) {
        let var_name = match &receiver {
            Expr::Variable(name) => name.clone(),
            Expr::AttributeAccess { variable, .. } => variable.clone(),
            _ => {
                return Err(ReaperError::InvalidPolicy {
                    reason: "Invalid receiver for attribute access".to_string(),
                })
            }
        };

        receiver = Expr::AttributeAccess {
            variable: var_name,
            attribute: attr.clone(),
        };
    }

    // Apply the method chain to the receiver
    parse_comp_method_chain(receiver, chain)
}

/// Parse function call in comprehension: is_string(x), concat(a, b), time.now_ns()
pub(super) fn parse_comp_function_call(
    pair: pest::iterators::Pair<Rule>,
) -> Result<Expr, ReaperError> {
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
