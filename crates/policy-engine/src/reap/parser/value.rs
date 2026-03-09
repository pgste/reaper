//! Value and attribute parsing for .reap files.
//!
//! This module handles parsing of:
//! - Entity attributes (user.name, resource.id)
//! - Variable attributes (var.attr)
//! - Bracket indexing ([0], ["key"], [_])
//! - Values (strings, integers, floats, booleans, null, arrays, objects, sets)

use super::Rule;
use crate::reap::ast::*;
use reaper_core::ReaperError;

/// Parse entity attribute access: user.name, resource.id[0]
pub(super) fn parse_entity_attr(
    pair: pest::iterators::Pair<Rule>,
) -> Result<EntityAttr, ReaperError> {
    let mut inner = pair.into_inner();
    let entity = Entity::from(inner.next().unwrap().as_str());

    // Collect all attribute identifiers (supports chained attributes like user.data.field.subfield)
    let mut attributes = Vec::new();
    let mut index = None;

    for item in inner {
        match item.as_rule() {
            Rule::ident => {
                attributes.push(item.as_str());
            }
            Rule::bracket_index => {
                let index_value_pair = item.into_inner().next().unwrap();
                index = Some(parse_bracket_index(index_value_pair)?);
            }
            _ => {}
        }
    }

    // Join attributes with dots
    let attribute = attributes.join(".");

    Ok(EntityAttr {
        entity,
        attribute,
        index,
    })
}

/// Parse variable attribute access: var.attr, var.attr[0]
pub(super) fn parse_var_attr(pair: pest::iterators::Pair<Rule>) -> Result<VarAttr, ReaperError> {
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

/// Parse bracket index: [0], ["key"], [_]
pub(super) fn parse_bracket_index(pair: pest::iterators::Pair<Rule>) -> Result<Index, ReaperError> {
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

/// Parse a value: string, integer, float, boolean, null, array, object, set
pub(super) fn parse_value(pair: pest::iterators::Pair<Rule>) -> Result<Value, ReaperError> {
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

/// Parse array value: [1, 2, 3]
pub(super) fn parse_array(pair: pest::iterators::Pair<Rule>) -> Result<Value, ReaperError> {
    let mut values = Vec::new();

    for inner in pair.into_inner() {
        if inner.as_rule() == Rule::value {
            values.push(parse_value(inner)?);
        }
    }

    Ok(Value::Array(values))
}

/// Parse braced expression: object {key: value} or set {value1, value2}
pub(super) fn parse_braced_expr(pair: pest::iterators::Pair<Rule>) -> Result<Value, ReaperError> {
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

/// Parse string literal, removing quotes and handling escapes
pub(super) fn parse_string_literal(
    pair: pest::iterators::Pair<Rule>,
) -> Result<String, ReaperError> {
    // String is atomic (@), so we get the full string with quotes
    let s = pair.as_str();
    // Remove surrounding quotes
    let trimmed = &s[1..s.len() - 1];
    // Unescape if needed (simple implementation)
    Ok(trimmed.replace("\\\"", "\"").replace("\\\\", "\\"))
}
