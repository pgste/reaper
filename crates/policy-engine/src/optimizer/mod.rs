//! Policy Optimization Module
//!
//! Provides optimizations for policy evaluation:
//! - Decision trees for O(log r) evaluation
//! - Attribute-based routing
//! - Evaluation caching
//!
//! # Phase 5A: Decision Trees
//!
//! Compiles policies into optimized decision trees that enable
//! logarithmic-time evaluation regardless of policy size.

pub mod decision_tree;

pub use decision_tree::{DecisionTree, DecisionTreeBuilder, TreeStats};
