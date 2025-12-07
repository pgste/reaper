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
        left: ComparisonLeft,
        op: Operator,
        right: ComparisonRight,
    },
    /// Local variable assignment: x := user.role
    Assignment {
        variable: String,
        value: AssignmentValue,
    },
    /// AND of conditions
    And(Vec<Condition>),
    /// OR of conditions
    Or(Vec<Condition>),
    /// NOT of condition
    Not(Box<Condition>),
    /// Expression that evaluates to boolean (e.g., function calls like is_string(x))
    Expr(Expr),
}

/// Left side of comparison
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ComparisonLeft {
    EntityAttr(EntityAttr),
    VarAttr(VarAttr),
}

/// Value that can be assigned to a variable
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AssignmentValue {
    /// Entity attribute: user.role
    EntityAttr(EntityAttr),
    /// Literal value: "admin", 42, true
    Value(Value),
    /// Another variable reference: x
    Variable(String),
    /// Comprehension: {expr | iteration; filters}
    Comprehension(Comprehension),
}

/// Entity attribute reference
/// Supports simple access (user.role) and indexed access (user.roles[0], user.data["key"])
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityAttr {
    pub entity: Entity,
    pub attribute: String,
    /// Optional index for bracket notation: user.roles[0] or user.data["key"]
    pub index: Option<Index>,
}

/// Index for bracket notation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Index {
    /// Numeric index for arrays: [0], [1], [42]
    Number(i64),
    /// String key for objects: ["key"], ["some-key"]
    String(String),
    /// Wildcard for iteration: [_] - iterates over all elements (existential quantification)
    Wildcard,
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
    /// Membership test: "admin" in user.roles
    In,
}

/// Right side of comparison
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ComparisonRight {
    Value(Value),
    EntityAttr(EntityAttr),
    /// Variable reference: x
    Variable(String),
    /// Variable attribute access: u.name, u.roles[0]
    VarAttr(VarAttr),
}

/// Variable attribute reference (for comprehension filters)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VarAttr {
    pub variable: String,
    pub attribute: String,
    /// Optional index for bracket notation: u.roles[0] or u.data["key"]
    pub index: Option<Index>,
}

/// Literal value
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Value {
    String(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Null,
    /// Array of values: [1, 2, 3]
    Array(Vec<Value>),
    /// Object/map of key-value pairs: {"key": "value", "num": 42}
    /// Uses Vec to maintain insertion order (important for deterministic behavior)
    Object(Vec<(String, Value)>),
    /// Set of unique values: {"foo", "bar", "baz"}
    /// Parser stores as Vec, evaluator converts to HashSet
    Set(Vec<Value>),
}

/// Comprehension expression for collecting and transforming data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Comprehension {
    /// Set comprehension: {expr | iteration; filters}
    /// Produces a HashSet of unique values
    Set {
        output: Box<Expr>,
        iterator: ComprehensionIterator,
        filters: Vec<Condition>,
    },

    /// Array comprehension: [expr | iteration; filters]
    /// Produces a Vec that preserves order and allows duplicates
    Array {
        output: Box<Expr>,
        iterator: ComprehensionIterator,
        filters: Vec<Condition>,
    },

    /// Object comprehension: {key: value | iteration; filters}
    /// Produces a HashMap of key-value pairs
    Object {
        key: Box<Expr>,
        value: Box<Expr>,
        iterator: ComprehensionIterator,
        filters: Vec<Condition>,
    },
}

/// Iterator specification for comprehensions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComprehensionIterator {
    /// Variable name to bind each element to (e.g., "u" in "u := users[_]")
    pub variable: String,

    /// Collection to iterate over (e.g., "users[_]", "user.roles[_]")
    pub collection: EntityAttr,
}

/// Expression type for comprehension output
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Expr {
    /// Literal value: "admin", 42, true, null
    Literal(Value),

    /// Variable reference: u, role, x
    Variable(String),

    /// Attribute access: u.name, role.level
    /// Variable must be bound by iterator or previous assignment
    AttributeAccess { variable: String, attribute: String },

    /// Indexed access: u.roles[0], user.data["key"], perms[_]
    IndexedAccess {
        variable: String,
        attribute: String,
        index: Index,
    },

    /// Method call: users.count(), roles.sum(), name.lower()
    /// Enables Rust-style method chaining for better ergonomics
    MethodCall {
        receiver: Box<Expr>,
        method: MethodName,
        args: Vec<Expr>,
    },

    /// Function call: time.now_ns(), concat(a, b), is_string(x)
    /// Supports namespaced (time.now_ns) and global (concat) functions
    FunctionCall {
        namespace: Option<String>,
        function: String,
        args: Vec<Expr>,
    },
}

/// Method names for method call syntax
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MethodName {
    // Aggregate methods
    Count,
    Sum,
    Max,
    Min,
    Any,
    All,

    // String methods
    Lower,
    Upper,
    Trim,
    Split,
    Contains,
    Startswith,
    Endswith,

    // Collection methods
    Union,
    Intersection,
    Difference,
}

/// Type names for type checking functions
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(dead_code)]
pub enum TypeName {
    String,
    Number,
    Bool,
    Array,
    Set,
    Object,
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
            "in" => Operator::In,
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

impl MethodName {
    pub fn from_str(s: &str) -> Result<Self, String> {
        match s {
            // Aggregates
            "count" => Ok(MethodName::Count),
            "sum" => Ok(MethodName::Sum),
            "max" => Ok(MethodName::Max),
            "min" => Ok(MethodName::Min),
            "any" => Ok(MethodName::Any),
            "all" => Ok(MethodName::All),

            // Strings
            "lower" => Ok(MethodName::Lower),
            "upper" => Ok(MethodName::Upper),
            "trim" => Ok(MethodName::Trim),
            "split" => Ok(MethodName::Split),
            "contains" => Ok(MethodName::Contains),
            "startswith" => Ok(MethodName::Startswith),
            "endswith" => Ok(MethodName::Endswith),

            // Collections
            "union" => Ok(MethodName::Union),
            "intersection" => Ok(MethodName::Intersection),
            "difference" => Ok(MethodName::Difference),

            _ => Err(format!("Unknown method name: {}", s)),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            MethodName::Count => "count",
            MethodName::Sum => "sum",
            MethodName::Max => "max",
            MethodName::Min => "min",
            MethodName::Any => "any",
            MethodName::All => "all",
            MethodName::Lower => "lower",
            MethodName::Upper => "upper",
            MethodName::Trim => "trim",
            MethodName::Split => "split",
            MethodName::Contains => "contains",
            MethodName::Startswith => "startswith",
            MethodName::Endswith => "endswith",
            MethodName::Union => "union",
            MethodName::Intersection => "intersection",
            MethodName::Difference => "difference",
        }
    }
}

impl TypeName {
    #[allow(dead_code)]
    pub fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "string" => Ok(TypeName::String),
            "number" => Ok(TypeName::Number),
            "bool" => Ok(TypeName::Bool),
            "array" => Ok(TypeName::Array),
            "set" => Ok(TypeName::Set),
            "object" => Ok(TypeName::Object),
            "null" => Ok(TypeName::Null),
            _ => Err(format!("Unknown type name: {}", s)),
        }
    }

    #[allow(dead_code)]
    pub fn as_str(&self) -> &'static str {
        match self {
            TypeName::String => "string",
            TypeName::Number => "number",
            TypeName::Bool => "bool",
            TypeName::Array => "array",
            TypeName::Set => "set",
            TypeName::Object => "object",
            TypeName::Null => "null",
        }
    }
}
