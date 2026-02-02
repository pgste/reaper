//! String methods for AST evaluation.
//!
//! This module provides string operations:
//! - lower() - Convert to lowercase
//! - upper() - Convert to uppercase
//! - trim() - Remove whitespace
//! - split() - Split by delimiter
//! - contains() - Check substring/membership
//! - startswith() - Check prefix
//! - endswith() - Check suffix

use super::super::types::EvalValue;
use reaper_core::ReaperError;

/// lower() - Converts string to lowercase
#[inline]
pub fn method_lower(value: &EvalValue) -> Result<EvalValue, ReaperError> {
    match value {
        EvalValue::String(s) => Ok(EvalValue::String(s.to_lowercase())),
        _ => Err(ReaperError::InvalidPolicy {
            reason: "lower() requires string value".to_string(),
        }),
    }
}

/// upper() - Converts string to uppercase
#[inline]
pub fn method_upper(value: &EvalValue) -> Result<EvalValue, ReaperError> {
    match value {
        EvalValue::String(s) => Ok(EvalValue::String(s.to_uppercase())),
        _ => Err(ReaperError::InvalidPolicy {
            reason: "upper() requires string value".to_string(),
        }),
    }
}

/// trim() - Removes leading/trailing whitespace
#[inline]
pub fn method_trim(value: &EvalValue) -> Result<EvalValue, ReaperError> {
    match value {
        EvalValue::String(s) => Ok(EvalValue::String(s.trim().to_string())),
        _ => Err(ReaperError::InvalidPolicy {
            reason: "trim() requires string value".to_string(),
        }),
    }
}

/// split() - Splits string by delimiter
#[inline]
pub fn method_split(value: &EvalValue, delimiter: &EvalValue) -> Result<EvalValue, ReaperError> {
    match (value, delimiter) {
        (EvalValue::String(s), EvalValue::String(delim)) => {
            let parts: Vec<EvalValue> = s
                .split(delim.as_str())
                .map(|part| EvalValue::String(part.to_string()))
                .collect();
            Ok(EvalValue::Array(parts))
        }
        _ => Err(ReaperError::InvalidPolicy {
            reason: "split() requires string value and delimiter".to_string(),
        }),
    }
}

/// contains() - Checks if string contains substring or collection contains item
#[inline]
pub fn method_contains(value: &EvalValue, item: &EvalValue) -> Result<EvalValue, ReaperError> {
    match (value, item) {
        (EvalValue::String(s), EvalValue::String(sub)) => {
            Ok(EvalValue::Boolean(s.contains(sub.as_str())))
        }
        (EvalValue::Array(arr), item) | (EvalValue::Set(arr), item) => {
            Ok(EvalValue::Boolean(arr.contains(item)))
        }
        _ => Err(ReaperError::InvalidPolicy {
            reason: "contains() requires string/collection and value".to_string(),
        }),
    }
}

/// startswith() - Checks if string starts with prefix
#[inline]
pub fn method_startswith(value: &EvalValue, prefix: &EvalValue) -> Result<EvalValue, ReaperError> {
    match (value, prefix) {
        (EvalValue::String(s), EvalValue::String(pre)) => {
            Ok(EvalValue::Boolean(s.starts_with(pre.as_str())))
        }
        _ => Err(ReaperError::InvalidPolicy {
            reason: "startswith() requires string value and prefix".to_string(),
        }),
    }
}

/// endswith() - Checks if string ends with suffix
#[inline]
pub fn method_endswith(value: &EvalValue, suffix: &EvalValue) -> Result<EvalValue, ReaperError> {
    match (value, suffix) {
        (EvalValue::String(s), EvalValue::String(suf)) => {
            Ok(EvalValue::Boolean(s.ends_with(suf.as_str())))
        }
        _ => Err(ReaperError::InvalidPolicy {
            reason: "endswith() requires string value and suffix".to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lower() {
        assert_eq!(
            method_lower(&EvalValue::String("HELLO".to_string())).unwrap(),
            EvalValue::String("hello".to_string())
        );
    }

    #[test]
    fn test_upper() {
        assert_eq!(
            method_upper(&EvalValue::String("hello".to_string())).unwrap(),
            EvalValue::String("HELLO".to_string())
        );
    }

    #[test]
    fn test_trim() {
        assert_eq!(
            method_trim(&EvalValue::String("  hello  ".to_string())).unwrap(),
            EvalValue::String("hello".to_string())
        );
    }

    #[test]
    fn test_split() {
        let result = method_split(
            &EvalValue::String("a,b,c".to_string()),
            &EvalValue::String(",".to_string()),
        ).unwrap();

        if let EvalValue::Array(parts) = result {
            assert_eq!(parts.len(), 3);
            assert_eq!(parts[0], EvalValue::String("a".to_string()));
        } else {
            panic!("Expected array");
        }
    }

    #[test]
    fn test_contains() {
        assert_eq!(
            method_contains(
                &EvalValue::String("hello world".to_string()),
                &EvalValue::String("world".to_string())
            ).unwrap(),
            EvalValue::Boolean(true)
        );
    }

    #[test]
    fn test_startswith() {
        assert_eq!(
            method_startswith(
                &EvalValue::String("hello".to_string()),
                &EvalValue::String("hel".to_string())
            ).unwrap(),
            EvalValue::Boolean(true)
        );
    }

    #[test]
    fn test_endswith() {
        assert_eq!(
            method_endswith(
                &EvalValue::String("hello".to_string()),
                &EvalValue::String("lo".to_string())
            ).unwrap(),
            EvalValue::Boolean(true)
        );
    }
}
