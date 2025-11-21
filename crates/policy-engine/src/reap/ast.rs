// ! Abstract Syntax Tree for Reaper Policy Language

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Top-level policy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Policy {
    pub name: String,
    pub metadata: HashMap<String, String>,
    pub default_decision: Decision,
    pub rules: Vec<Rule>,
}

/// A single policy rule
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    pub name: String,
    pub decision: Decision,
    pub condition: Condition,
}

/// Decision type
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Decision {
    Allow,
    Deny,
}

/// Condition expression
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Condition {
    /// Always true
    True,
    /// Always false
    False,
    /// Comparison
    Comparison {
        left: EntityAttr,
        op: Operator,
        right: ComparisonRight,
    },
    /// AND of conditions
    And(Vec<Condition>),
    /// OR of conditions
    Or(Vec<Condition>),
    /// NOT of condition
    Not(Box<Condition>),
}

/// Entity attribute reference
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityAttr {
    pub entity: Entity,
    pub attribute: String,
}

/// Entity type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Entity {
    User,
    Resource,
    Context,
}

/// Comparison operator
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Operator {
    Equal,
    NotEqual,
    GreaterThan,
    LessThan,
    GreaterEqual,
    LessEqual,
}

/// Right side of comparison
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ComparisonRight {
    Value(Value),
    EntityAttr(EntityAttr),
}

/// Literal value
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Value {
    String(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Null,
}

impl From<&str> for Entity {
    fn from(s: &str) -> Self {
        match s {
            "user" => Entity::User,
            "resource" => Entity::Resource,
            "context" => Entity::Context,
            _ => panic!("Invalid entity type: {}", s),
        }
    }
}

impl From<&str> for Operator {
    fn from(s: &str) -> Self {
        match s {
            "==" => Operator::Equal,
            "!=" => Operator::NotEqual,
            ">" => Operator::GreaterThan,
            "<" => Operator::LessThan,
            ">=" => Operator::GreaterEqual,
            "<=" => Operator::LessEqual,
            _ => panic!("Invalid operator: {}", s),
        }
    }
}

impl From<&str> for Decision {
    fn from(s: &str) -> Self {
        match s {
            "allow" => Decision::Allow,
            "deny" => Decision::Deny,
            _ => panic!("Invalid decision: {}", s),
        }
    }
}
