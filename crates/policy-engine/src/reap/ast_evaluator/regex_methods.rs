//! Regex methods with caching for AST evaluator.
//!
//! This module provides regex operations with caching for 2-5x performance improvement:
//! - get_cached_regex: Compile and cache regex patterns
//! - method_matches: Test if string matches regex
//! - method_find: Find first match
//! - method_find_all: Find all matches
//! - method_replace: Replace all matches

use super::types::EvalValue;
use super::ReapAstEvaluator;
use reaper_core::ReaperError;

impl ReapAstEvaluator {
    /// Get or compile a regex pattern with caching for 2-5x performance improvement
    ///
    /// Regex compilation is expensive (~1-10 µs per pattern).
    /// Caching provides significant speedup when the same pattern is used multiple times.
    pub(super) fn get_cached_regex(&self, pattern: &str) -> Result<regex::Regex, ReaperError> {
        // Fast path: check if already cached
        {
            let cache = self.regex_cache.lock();
            if let Some(re) = cache.get(pattern) {
                return Ok(re.clone());
            }
        } // Release lock before compiling (slow operation)

        // Slow path: compile regex (outside lock to avoid holding lock during compilation)
        let re = regex::Regex::new(pattern).map_err(|e| ReaperError::InvalidPolicy {
            reason: format!("Invalid regex pattern '{}': {}", pattern, e),
        })?;

        // Insert into cache for future use
        {
            let mut cache = self.regex_cache.lock();
            cache.insert(pattern.to_string(), re.clone());
        }

        Ok(re)
    }

    /// matches() - Tests if string matches regex pattern (with caching)
    pub(super) fn method_matches(
        &self,
        value: &EvalValue,
        pattern: &EvalValue,
    ) -> Result<EvalValue, ReaperError> {
        match (value, pattern) {
            (EvalValue::String(s), EvalValue::String(pat)) => {
                let re = self.get_cached_regex(pat)?;
                Ok(EvalValue::Boolean(re.is_match(s)))
            }
            _ => Err(ReaperError::InvalidPolicy {
                reason: "matches() requires string value and pattern".to_string(),
            }),
        }
    }

    /// find() - Finds first match of regex pattern in string (with caching)
    pub(super) fn method_find(
        &self,
        value: &EvalValue,
        pattern: &EvalValue,
    ) -> Result<EvalValue, ReaperError> {
        match (value, pattern) {
            (EvalValue::String(s), EvalValue::String(pat)) => {
                let re = self.get_cached_regex(pat)?;

                match re.find(s) {
                    Some(m) => Ok(EvalValue::String(m.as_str().to_string())),
                    None => Ok(EvalValue::Null),
                }
            }
            _ => Err(ReaperError::InvalidPolicy {
                reason: "find() requires string value and pattern".to_string(),
            }),
        }
    }

    /// find_all() - Finds all matches of regex pattern in string (with caching)
    pub(super) fn method_find_all(
        &self,
        value: &EvalValue,
        pattern: &EvalValue,
    ) -> Result<EvalValue, ReaperError> {
        match (value, pattern) {
            (EvalValue::String(s), EvalValue::String(pat)) => {
                let re = self.get_cached_regex(pat)?;

                let matches: Vec<EvalValue> = re
                    .find_iter(s)
                    .map(|m| EvalValue::String(m.as_str().to_string()))
                    .collect();

                Ok(EvalValue::Array(matches))
            }
            _ => Err(ReaperError::InvalidPolicy {
                reason: "find_all() requires string value and pattern".to_string(),
            }),
        }
    }

    /// replace() - Replaces all matches of regex pattern with replacement string (with caching)
    pub(super) fn method_replace(
        &self,
        value: &EvalValue,
        pattern: &EvalValue,
        replacement: &EvalValue,
    ) -> Result<EvalValue, ReaperError> {
        match (value, pattern, replacement) {
            (EvalValue::String(s), EvalValue::String(pat), EvalValue::String(rep)) => {
                let re = self.get_cached_regex(pat)?;

                let result = re.replace_all(s, rep.as_str()).to_string();
                Ok(EvalValue::String(result))
            }
            _ => Err(ReaperError::InvalidPolicy {
                reason: "replace() requires string value, pattern, and replacement".to_string(),
            }),
        }
    }
}
