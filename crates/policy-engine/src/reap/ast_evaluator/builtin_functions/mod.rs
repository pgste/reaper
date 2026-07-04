//! Builtin functions for AST evaluation.
//!
//! This module organizes built-in namespace functions into categories:
//! - time: now_ns, now_ms, now, parse_rfc3339, format_rfc3339, add_ns, subtract_ns, is_before, is_after, is_between
//! - math: abs, round, floor, ceil, sqrt, pow, min, max, clamp
//! - regex: escape (cache-dependent functions remain in evaluator)
//! - json: parse, stringify, is_valid
//! - type_check: is_string, is_number, is_bool, is_array, is_set, is_object, is_null, concat

pub(super) mod json;
pub(super) mod jwt;
pub(super) mod math;
pub(super) mod regex;
pub(super) mod time;
pub(super) mod type_check;

// Re-export time functions
pub(super) use time::{
    add_ns as time_add_ns, format_rfc3339 as time_format_rfc3339, is_after as time_is_after,
    is_before as time_is_before, is_between as time_is_between, now as time_now,
    now_ms as time_now_ms, now_ns as time_now_ns, parse_rfc3339 as time_parse_rfc3339,
    subtract_ns as time_subtract_ns,
};

// Re-export math functions
pub(super) use math::{
    abs as math_abs, ceil as math_ceil, clamp as math_clamp, floor as math_floor, max as math_max,
    min as math_min, pow as math_pow, round as math_round, sqrt as math_sqrt,
};

// Re-export regex functions
pub(super) use regex::escape as regex_escape;

// Re-export json functions
pub(super) use json::{
    is_valid as json_is_valid, parse as json_parse, stringify as json_stringify,
};

// Re-export type check functions
pub(super) use type_check::{
    concat, is_array, is_bool, is_null, is_number, is_object, is_set, is_string,
};
