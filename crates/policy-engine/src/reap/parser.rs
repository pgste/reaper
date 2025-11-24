// ! Parser for .reap files using Pest

use pest::Parser as PestParser;
use pest_derive::Parser;
use reaper_core::ReaperError;
use super::ast::*;
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

fn parse_metadata_field(pair: pest::iterators::Pair<Rule>) -> Result<(String, String), ReaperError> {
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
        Rule::comparison => parse_comparison(inner),
        Rule::boolean_literal => {
            match inner.as_str() {
                "true" => Ok(Condition::True),
                "false" => Ok(Condition::False),
                _ => Err(ReaperError::InvalidPolicy {
                    reason: format!("Invalid boolean literal: {}", inner.as_str()),
                }),
            }
        }
        _ => Err(ReaperError::InvalidPolicy {
            reason: format!("Unexpected rule in primary_expr: {:?}", inner.as_rule()),
        }),
    }
}

fn parse_comparison(pair: pest::iterators::Pair<Rule>) -> Result<Condition, ReaperError> {
    let mut inner = pair.into_inner();

    let left = parse_entity_attr(inner.next().unwrap())?;
    let op = Operator::from(inner.next().unwrap().as_str());
    let right_pair = inner.next().unwrap();

    let right = match right_pair.as_rule() {
        Rule::entity_attr => ComparisonRight::EntityAttr(parse_entity_attr(right_pair)?),
        Rule::value => ComparisonRight::Value(parse_value(right_pair)?),
        _ => return Err(ReaperError::InvalidPolicy {
            reason: format!("Unexpected right side: {:?}", right_pair.as_rule()),
        }),
    };

    Ok(Condition::Comparison { left, op, right })
}

fn parse_entity_attr(pair: pest::iterators::Pair<Rule>) -> Result<EntityAttr, ReaperError> {
    let mut inner = pair.into_inner();
    let entity = Entity::from(inner.next().unwrap().as_str());
    let attribute = inner.next().unwrap().as_str().to_string();

    Ok(EntityAttr { entity, attribute })
}

fn parse_value(pair: pest::iterators::Pair<Rule>) -> Result<Value, ReaperError> {
    let inner = pair.into_inner().next().unwrap();

    match inner.as_rule() {
        Rule::string => Ok(Value::String(parse_string_literal(inner)?)),
        Rule::integer => {
            let val = inner.as_str().parse::<i64>().map_err(|e| {
                ReaperError::InvalidPolicy {
                    reason: format!("Invalid integer: {}", e),
                }
            })?;
            Ok(Value::Integer(val))
        }
        Rule::float => {
            let val = inner.as_str().parse::<f64>().map_err(|e| {
                ReaperError::InvalidPolicy {
                    reason: format!("Invalid float: {}", e),
                }
            })?;
            Ok(Value::Float(val))
        }
        Rule::boolean_literal => {
            let val = inner.as_str() == "true";
            Ok(Value::Boolean(val))
        }
        Rule::null_literal => Ok(Value::Null),
        _ => Err(ReaperError::InvalidPolicy {
            reason: format!("Unexpected value type: {:?}", inner.as_rule()),
        }),
    }
}

fn parse_string_literal(pair: pest::iterators::Pair<Rule>) -> Result<String, ReaperError> {
    // String is atomic (@), so we get the full string with quotes
    let s = pair.as_str();
    // Remove surrounding quotes
    let trimmed = &s[1..s.len()-1];
    // Unescape if needed (simple implementation)
    Ok(trimmed.replace("\\\"", "\"").replace("\\\\", "\\"))
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
}
