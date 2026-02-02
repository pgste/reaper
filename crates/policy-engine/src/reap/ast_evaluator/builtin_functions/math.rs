//! Math functions for policy evaluation.
//!
//! This module provides mathematical operations:
//! - abs() - Absolute value
//! - round(), floor(), ceil() - Rounding functions
//! - sqrt(), pow() - Power functions
//! - min(), max(), clamp() - Range functions

use super::super::types::EvalValue;
use reaper_core::ReaperError;

/// math::abs(n) - Returns absolute value of a number
#[inline]
pub fn abs(value: &EvalValue) -> Result<EvalValue, ReaperError> {
    match value {
        EvalValue::Integer(n) => Ok(EvalValue::Integer(n.abs())),
        EvalValue::Float(f) => Ok(EvalValue::Float(f.abs())),
        _ => Err(ReaperError::InvalidPolicy {
            reason: "math::abs() requires numeric argument".to_string(),
        }),
    }
}

/// math::round(n) - Round to nearest integer
#[inline]
pub fn round(value: &EvalValue) -> Result<EvalValue, ReaperError> {
    let num = match value {
        EvalValue::Integer(n) => *n as f64,
        EvalValue::Float(f) => *f,
        _ => {
            return Err(ReaperError::InvalidPolicy {
                reason: "math::round() requires numeric argument".to_string(),
            })
        }
    };
    Ok(EvalValue::Integer(num.round() as i64))
}

/// math::floor(n) - Round down to integer
#[inline]
pub fn floor(value: &EvalValue) -> Result<EvalValue, ReaperError> {
    let num = match value {
        EvalValue::Integer(n) => *n as f64,
        EvalValue::Float(f) => *f,
        _ => {
            return Err(ReaperError::InvalidPolicy {
                reason: "math::floor() requires numeric argument".to_string(),
            })
        }
    };
    Ok(EvalValue::Integer(num.floor() as i64))
}

/// math::ceil(n) - Round up to integer
#[inline]
pub fn ceil(value: &EvalValue) -> Result<EvalValue, ReaperError> {
    let num = match value {
        EvalValue::Integer(n) => *n as f64,
        EvalValue::Float(f) => *f,
        _ => {
            return Err(ReaperError::InvalidPolicy {
                reason: "math::ceil() requires numeric argument".to_string(),
            })
        }
    };
    Ok(EvalValue::Integer(num.ceil() as i64))
}

/// math::sqrt(n) - Square root
#[inline]
pub fn sqrt(value: &EvalValue) -> Result<EvalValue, ReaperError> {
    let num = match value {
        EvalValue::Integer(n) => *n as f64,
        EvalValue::Float(f) => *f,
        _ => {
            return Err(ReaperError::InvalidPolicy {
                reason: "math::sqrt() requires numeric argument".to_string(),
            })
        }
    };
    if num < 0.0 {
        return Err(ReaperError::InvalidPolicy {
            reason: "math::sqrt() requires non-negative argument".to_string(),
        });
    }
    Ok(EvalValue::Float(num.sqrt()))
}

/// math::pow(base, exponent) - Power function
#[inline]
pub fn pow(base: &EvalValue, exp: &EvalValue) -> Result<EvalValue, ReaperError> {
    let base_val = match base {
        EvalValue::Integer(n) => *n as f64,
        EvalValue::Float(f) => *f,
        _ => {
            return Err(ReaperError::InvalidPolicy {
                reason: "math::pow() base must be numeric".to_string(),
            })
        }
    };
    let exp_val = match exp {
        EvalValue::Integer(n) => *n as f64,
        EvalValue::Float(f) => *f,
        _ => {
            return Err(ReaperError::InvalidPolicy {
                reason: "math::pow() exponent must be numeric".to_string(),
            })
        }
    };
    let result = base_val.powf(exp_val);
    // Return integer if result is a whole number and exponent was non-negative
    if exp_val >= 0.0 && result.fract() == 0.0 && result.is_finite() {
        Ok(EvalValue::Integer(result as i64))
    } else {
        Ok(EvalValue::Float(result))
    }
}

/// math::min(a, b) - Minimum of two values
#[inline]
pub fn min(a: &EvalValue, b: &EvalValue) -> Result<EvalValue, ReaperError> {
    match (a, b) {
        (EvalValue::Integer(x), EvalValue::Integer(y)) => Ok(EvalValue::Integer(*x.min(y))),
        (EvalValue::Float(x), EvalValue::Float(y)) => Ok(EvalValue::Float(x.min(*y))),
        (EvalValue::Integer(x), EvalValue::Float(y))
        | (EvalValue::Float(y), EvalValue::Integer(x)) => {
            Ok(EvalValue::Float((*x as f64).min(*y)))
        }
        _ => Err(ReaperError::InvalidPolicy {
            reason: "math::min() requires two numeric arguments".to_string(),
        }),
    }
}

/// math::max(a, b) - Maximum of two values
#[inline]
pub fn max(a: &EvalValue, b: &EvalValue) -> Result<EvalValue, ReaperError> {
    match (a, b) {
        (EvalValue::Integer(x), EvalValue::Integer(y)) => Ok(EvalValue::Integer(*x.max(y))),
        (EvalValue::Float(x), EvalValue::Float(y)) => Ok(EvalValue::Float(x.max(*y))),
        (EvalValue::Integer(x), EvalValue::Float(y))
        | (EvalValue::Float(y), EvalValue::Integer(x)) => {
            Ok(EvalValue::Float((*x as f64).max(*y)))
        }
        _ => Err(ReaperError::InvalidPolicy {
            reason: "math::max() requires two numeric arguments".to_string(),
        }),
    }
}

/// math::clamp(value, min, max) - Clamp value to range
#[inline]
pub fn clamp(
    value: &EvalValue,
    min_val: &EvalValue,
    max_val: &EvalValue,
) -> Result<EvalValue, ReaperError> {
    let val = match value {
        EvalValue::Integer(n) => *n as f64,
        EvalValue::Float(f) => *f,
        _ => {
            return Err(ReaperError::InvalidPolicy {
                reason: "math::clamp() value must be numeric".to_string(),
            })
        }
    };
    let min_f = match min_val {
        EvalValue::Integer(n) => *n as f64,
        EvalValue::Float(f) => *f,
        _ => {
            return Err(ReaperError::InvalidPolicy {
                reason: "math::clamp() min must be numeric".to_string(),
            })
        }
    };
    let max_f = match max_val {
        EvalValue::Integer(n) => *n as f64,
        EvalValue::Float(f) => *f,
        _ => {
            return Err(ReaperError::InvalidPolicy {
                reason: "math::clamp() max must be numeric".to_string(),
            })
        }
    };

    if min_f > max_f {
        return Err(ReaperError::InvalidPolicy {
            reason: "math::clamp() min must be <= max".to_string(),
        });
    }

    let clamped = val.clamp(min_f, max_f);

    // Return integer if all inputs were integers
    let all_integers = matches!(value, EvalValue::Integer(_))
        && matches!(min_val, EvalValue::Integer(_))
        && matches!(max_val, EvalValue::Integer(_));

    if all_integers {
        Ok(EvalValue::Integer(clamped as i64))
    } else {
        Ok(EvalValue::Float(clamped))
    }
}
