//! Builtin methods for AST evaluation.
//!
//! This module organizes built-in methods into categories:
//! - aggregate: count, sum, max, min, any, all
//! - string: lower, upper, trim, split, contains, startswith, endswith
//! - collection: first, last, slice, reverse, sort, unique
//! - set_ops: union, intersection, difference
//! - object: keys, values, has_key

pub mod aggregate;
pub mod collection;
pub mod object;
pub mod set_ops;
pub mod string;

// Re-export commonly used functions
pub use aggregate::{method_all, method_any, method_count, method_max, method_min, method_sum};
pub use collection::{
    get_collection_items, method_first, method_last, method_reverse, method_slice, method_sort,
    method_unique,
};
pub use object::{method_has_key, method_keys, method_values};
pub use set_ops::{method_difference, method_intersection, method_union};
pub use string::{
    method_contains, method_endswith, method_lower, method_split, method_startswith, method_trim,
    method_upper,
};
