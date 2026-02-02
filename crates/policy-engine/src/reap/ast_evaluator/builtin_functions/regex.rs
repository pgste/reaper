//! Regex functions for policy evaluation.
//!
//! This module provides regex-related operations:
//! - escape() - Escape special regex characters
//!
//! Note: Functions that require regex caching (is_valid, matches, replace, split)
//! remain in the evaluator mod.rs as they need access to the regex cache.

// Allow unused helper functions - reserved for future non-cached use cases
#![allow(dead_code)]

use super::super::types::EvalValue;
use reaper_core::ReaperError;

/// regex::escape(string) - Escape special regex characters in a string
#[inline]
pub fn escape(value: &EvalValue) -> Result<EvalValue, ReaperError> {
    let input = match value {
        EvalValue::String(s) => s,
        _ => {
            return Err(ReaperError::InvalidPolicy {
                reason: "regex::escape() argument must be a string".to_string(),
            })
        }
    };

    Ok(EvalValue::String(regex::escape(input)))
}

/// Helper to validate a regex pattern
/// Returns true if the pattern is valid, false otherwise
#[inline]
pub fn is_valid_pattern(pattern: &str) -> bool {
    regex::Regex::new(pattern).is_ok()
}

/// Helper to compile a regex and check if text matches
/// For use when caching is not available
#[inline]
pub fn matches_uncached(text: &str, pattern: &str) -> Result<bool, ReaperError> {
    let re = regex::Regex::new(pattern).map_err(|e| ReaperError::InvalidPolicy {
        reason: format!("Invalid regex pattern '{}': {}", pattern, e),
    })?;
    Ok(re.is_match(text))
}

/// Helper to compile a regex and replace all matches
/// For use when caching is not available
#[inline]
pub fn replace_uncached(text: &str, pattern: &str, replacement: &str) -> Result<String, ReaperError> {
    let re = regex::Regex::new(pattern).map_err(|e| ReaperError::InvalidPolicy {
        reason: format!("Invalid regex pattern '{}': {}", pattern, e),
    })?;
    Ok(re.replace_all(text, replacement).to_string())
}

/// Helper to compile a regex and split text
/// For use when caching is not available
#[inline]
pub fn split_uncached(text: &str, pattern: &str) -> Result<Vec<String>, ReaperError> {
    let re = regex::Regex::new(pattern).map_err(|e| ReaperError::InvalidPolicy {
        reason: format!("Invalid regex pattern '{}': {}", pattern, e),
    })?;
    Ok(re.split(text).map(|s| s.to_string()).collect())
}
