// YAML/JSON Parser for Reaper Policies
//
// Parses YAML and JSON policy definitions into the Reaper AST.
// Provides better error messages and validation than raw serde deserialization.

use super::ast::*;
use reaper_core::ReaperError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// YAML/JSON policy document schema
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YamlPolicy {
    pub name: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    pub default_decision: String, // "allow" or "deny"
    pub rules: Vec<YamlRule>,
}

/// YAML/JSON rule schema
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YamlRule {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub decision: String, // "allow" or "deny"
    pub condition: YamlCondition,
}

/// YAML/JSON condition schema
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum YamlCondition {
    /// Comparison condition
    Comparison {
        operator: String,
        left: YamlEntityAttr,
        right: YamlComparisonRight,
    },
    /// Logical operator (and/or)
    Logical {
        operator: String,
        conditions: Vec<YamlCondition>,
    },
}

/// YAML/JSON entity attribute reference
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YamlEntityAttr {
    pub entity: String, // "user", "resource", "context"
    pub attribute: String,
}

/// YAML/JSON comparison right side
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum YamlComparisonRight {
    /// Literal value
    Value { value: serde_json::Value },
    /// Entity attribute
    EntityAttr { entity: String, attribute: String },
}

impl YamlPolicy {
    /// Parse from YAML string
    pub fn from_yaml(yaml: &str) -> Result<Self, ReaperError> {
        serde_yaml::from_str(yaml).map_err(|e| ReaperError::InvalidPolicy {
            reason: format!("Failed to parse YAML: {}", e),
        })
    }

    /// Parse from JSON string
    pub fn from_json(json: &str) -> Result<Self, ReaperError> {
        serde_json::from_str(json).map_err(|e| ReaperError::InvalidPolicy {
            reason: format!("Failed to parse JSON: {}", e),
        })
    }

    /// Convert to AST Policy
    pub fn to_ast(self) -> Result<Policy, ReaperError> {
        // Parse default decision
        let default_decision = parse_decision(&self.default_decision)?;

        // Build metadata
        let mut metadata = HashMap::new();
        if let Some(version) = self.version {
            metadata.insert("version".to_string(), version);
        }
        if let Some(description) = self.description {
            metadata.insert("description".to_string(), description);
        }

        // Convert rules
        let mut rules = Vec::new();
        for yaml_rule in self.rules {
            rules.push(yaml_rule.into_ast()?);
        }

        Ok(Policy {
            name: self.name,
            metadata,
            default_decision,
            rules,
        })
    }
}

impl YamlRule {
    /// Convert to AST Rule
    pub fn into_ast(self) -> Result<Rule, ReaperError> {
        let decision = parse_decision(&self.decision)?;
        let condition = self.condition.into_ast()?;

        Ok(Rule {
            message: None,
            name: self.name,
            decision,
            condition,
        })
    }
}

impl YamlCondition {
    /// Convert to AST Condition
    pub fn into_ast(self) -> Result<Condition, ReaperError> {
        match self {
            YamlCondition::Comparison {
                operator,
                left,
                right,
            } => {
                let op = parse_operator(&operator)?;
                let left_attr = left.into_ast()?;
                let right_value = right.into_ast()?;

                Ok(Condition::Comparison {
                    left: ComparisonLeft::EntityAttr(left_attr),
                    op,
                    right: right_value,
                })
            }
            YamlCondition::Logical {
                operator,
                conditions,
            } => {
                let conds: Result<Vec<_>, _> =
                    conditions.into_iter().map(|c| c.into_ast()).collect();
                let conds = conds?;

                match operator.to_lowercase().as_str() {
                    "and" => Ok(Condition::And(conds)),
                    "or" => Ok(Condition::Or(conds)),
                    _ => Err(ReaperError::InvalidPolicy {
                        reason: format!(
                            "Invalid logical operator '{}'. Use 'and' or 'or'",
                            operator
                        ),
                    }),
                }
            }
        }
    }
}

impl YamlEntityAttr {
    /// Convert to AST EntityAttr
    pub fn into_ast(self) -> Result<EntityAttr, ReaperError> {
        let entity = parse_entity(&self.entity)?;
        Ok(EntityAttr {
            entity,
            attribute: self.attribute,
            index: None,
        })
    }
}

impl YamlComparisonRight {
    /// Convert to AST ComparisonRight
    pub fn into_ast(self) -> Result<ComparisonRight, ReaperError> {
        match self {
            YamlComparisonRight::Value { value } => {
                let v = json_value_to_ast_value(value)?;
                Ok(ComparisonRight::Value(v))
            }
            YamlComparisonRight::EntityAttr { entity, attribute } => {
                let entity = parse_entity(&entity)?;
                Ok(ComparisonRight::EntityAttr(EntityAttr {
                    entity,
                    attribute,
                    index: None,
                }))
            }
        }
    }
}

// Helper functions

fn parse_decision(s: &str) -> Result<Decision, ReaperError> {
    match s.to_lowercase().as_str() {
        "allow" => Ok(Decision::Allow),
        "deny" => Ok(Decision::Deny),
        _ => Err(ReaperError::InvalidPolicy {
            reason: format!("Invalid decision '{}'. Use 'allow' or 'deny'", s),
        }),
    }
}

fn parse_entity(s: &str) -> Result<Entity, ReaperError> {
    match s.to_lowercase().as_str() {
        "user" => Ok(Entity::User),
        "actor" => Ok(Entity::Actor),
        "resource" => Ok(Entity::Resource),
        "context" => Ok(Entity::Context),
        "input" => Ok(Entity::Input),
        _ => Err(ReaperError::InvalidPolicy {
            reason: format!(
                "Invalid entity '{}'. Use 'user', 'resource', or 'context'",
                s
            ),
        }),
    }
}

fn parse_operator(s: &str) -> Result<Operator, ReaperError> {
    match s.to_lowercase().as_str() {
        "equal" | "eq" | "==" => Ok(Operator::Equal),
        "not_equal" | "ne" | "!=" => Ok(Operator::NotEqual),
        "greater_than" | "gt" | ">" => Ok(Operator::GreaterThan),
        "less_than" | "lt" | "<" => Ok(Operator::LessThan),
        "greater_equal" | "gte" | ">=" => Ok(Operator::GreaterEqual),
        "less_equal" | "lte" | "<=" => Ok(Operator::LessEqual),
        _ => Err(ReaperError::InvalidPolicy {
            reason: format!(
                "Invalid operator '{}'. Supported: equal, not_equal, greater_than, less_than, greater_equal, less_equal",
                s
            ),
        }),
    }
}

fn json_value_to_ast_value(v: serde_json::Value) -> Result<Value, ReaperError> {
    match v {
        serde_json::Value::String(s) => Ok(Value::String(s)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(Value::Integer(i))
            } else if let Some(f) = n.as_f64() {
                Ok(Value::Float(f))
            } else {
                Err(ReaperError::InvalidPolicy {
                    reason: format!("Invalid number value: {}", n),
                })
            }
        }
        serde_json::Value::Bool(b) => Ok(Value::Boolean(b)),
        serde_json::Value::Null => Ok(Value::Null),
        _ => Err(ReaperError::InvalidPolicy {
            reason: format!(
                "Invalid value type: {:?}. Use string, number, boolean, or null",
                v
            ),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_yaml() {
        let yaml = r#"
name: test_policy
version: "1.0.0"
description: "Test policy"
default_decision: deny

rules:
  - name: admin_access
    decision: allow
    condition:
      operator: equal
      left:
        entity: user
        attribute: role
      right:
        value: "admin"
"#;

        let policy = YamlPolicy::from_yaml(yaml).unwrap();
        assert_eq!(policy.name, "test_policy");
        assert_eq!(policy.rules.len(), 1);
        assert_eq!(policy.rules[0].name, "admin_access");

        let ast = policy.to_ast().unwrap();
        assert_eq!(ast.name, "test_policy");
        assert_eq!(ast.rules.len(), 1);
    }

    #[test]
    fn test_parse_simple_json() {
        let json = r#"
{
  "name": "test_policy",
  "version": "1.0.0",
  "default_decision": "deny",
  "rules": [
    {
      "name": "admin_access",
      "decision": "allow",
      "condition": {
        "operator": "equal",
        "left": {"entity": "user", "attribute": "role"},
        "right": {"value": "admin"}
      }
    }
  ]
}
"#;

        let policy = YamlPolicy::from_json(json).unwrap();
        assert_eq!(policy.name, "test_policy");
        assert_eq!(policy.rules.len(), 1);

        let ast = policy.to_ast().unwrap();
        assert_eq!(ast.name, "test_policy");
    }

    #[test]
    fn test_parse_and_condition() {
        let yaml = r#"
name: test_policy
default_decision: deny

rules:
  - name: manager_reports
    decision: allow
    condition:
      operator: and
      conditions:
        - operator: equal
          left: {entity: user, attribute: role}
          right: {value: "manager"}
        - operator: equal
          left: {entity: resource, attribute: type}
          right: {value: "report"}
"#;

        let policy = YamlPolicy::from_yaml(yaml).unwrap();
        let ast = policy.to_ast().unwrap();

        assert_eq!(ast.rules.len(), 1);
        match &ast.rules[0].condition {
            Condition::And(conds) => assert_eq!(conds.len(), 2),
            _ => panic!("Expected And condition"),
        }
    }

    #[test]
    fn test_parse_attribute_comparison() {
        let yaml = r#"
name: test_policy
default_decision: deny

rules:
  - name: owner_access
    decision: allow
    condition:
      operator: equal
      left:
        entity: user
        attribute: id
      right:
        entity: resource
        attribute: owner_id
"#;

        let policy = YamlPolicy::from_yaml(yaml).unwrap();
        let ast = policy.to_ast().unwrap();

        match &ast.rules[0].condition {
            Condition::Comparison { left, op, right } => {
                match left {
                    ComparisonLeft::EntityAttr(attr) => {
                        assert_eq!(attr.entity, Entity::User);
                        assert_eq!(attr.attribute, "id");
                    }
                    _ => panic!("Expected EntityAttr on left"),
                }
                assert_eq!(*op, Operator::Equal);
                match right {
                    ComparisonRight::EntityAttr(attr) => {
                        assert_eq!(attr.entity, Entity::Resource);
                        assert_eq!(attr.attribute, "owner_id");
                    }
                    _ => panic!("Expected EntityAttr on right"),
                }
            }
            _ => panic!("Expected Comparison condition"),
        }
    }

    #[test]
    fn test_invalid_decision() {
        let yaml = r#"
name: test_policy
default_decision: invalid
rules: []
"#;

        let policy = YamlPolicy::from_yaml(yaml).unwrap();
        assert!(policy.to_ast().is_err());
    }

    #[test]
    fn test_invalid_operator() {
        let yaml = r#"
name: test_policy
default_decision: deny
rules:
  - name: test
    decision: allow
    condition:
      operator: invalid_op
      left: {entity: user, attribute: role}
      right: {value: "admin"}
"#;

        let policy = YamlPolicy::from_yaml(yaml).unwrap();
        assert!(policy.to_ast().is_err());
    }
}
