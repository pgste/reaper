//! Time/Date functions for policy evaluation.
//!
//! This module provides time-related operations:
//! - now_ns(), now_ms(), now() - Current time functions
//! - parse_rfc3339(), format_rfc3339() - Time parsing/formatting
//! - add_ns(), subtract_ns() - Time arithmetic
//! - is_before(), is_after(), is_between() - Time comparisons

use super::super::types::EvalValue;
use reaper_core::ReaperError;

/// Current unix nanos via the target-portable clock (`crate::clock`), failing
/// closed when no clock is available (bare wasm with no injected time).
#[inline]
fn clock_now_ns() -> Result<i64, ReaperError> {
    crate::clock::now_unix_ns().ok_or_else(|| ReaperError::InvalidPolicy {
        reason: "System time error: no clock available on this target \
                 (inject one via clock::set_injected_now_unix_ns)"
            .to_string(),
    })
}

/// time::now_ns() - Returns current time in nanoseconds since Unix epoch
#[inline]
pub fn now_ns() -> Result<EvalValue, ReaperError> {
    Ok(EvalValue::Integer(clock_now_ns()?))
}

/// time::now_ms() - Returns current time in milliseconds since Unix epoch
#[inline]
/// Current unix time in SECONDS — the unit JWT `exp`/`nbf`/`iat` use.
pub fn now_secs() -> Result<EvalValue, ReaperError> {
    Ok(EvalValue::Integer(clock_now_ns()? / 1_000_000_000))
}

pub fn now_ms() -> Result<EvalValue, ReaperError> {
    Ok(EvalValue::Integer(clock_now_ns()? / 1_000_000))
}

/// time::now() - Returns current time in seconds since Unix epoch
#[inline]
pub fn now() -> Result<EvalValue, ReaperError> {
    Ok(EvalValue::Integer(clock_now_ns()? / 1_000_000_000))
}

/// time::parse_rfc3339(string) - Parse RFC3339/ISO8601 timestamp to nanoseconds
#[inline]
pub fn parse_rfc3339(value: &EvalValue) -> Result<EvalValue, ReaperError> {
    let time_str = match value {
        EvalValue::String(s) => s,
        _ => {
            return Err(ReaperError::InvalidPolicy {
                reason: "time::parse_rfc3339() requires a string argument".to_string(),
            })
        }
    };

    use chrono::DateTime;
    let dt = DateTime::parse_from_rfc3339(time_str).map_err(|e| ReaperError::InvalidPolicy {
        reason: format!("Invalid RFC3339 timestamp '{}': {}", time_str, e),
    })?;

    Ok(EvalValue::Integer(dt.timestamp_nanos_opt().unwrap_or(0)))
}

/// time::format_rfc3339(nanoseconds) - Format nanoseconds as RFC3339 string
#[inline]
pub fn format_rfc3339(value: &EvalValue) -> Result<EvalValue, ReaperError> {
    let nanos = match value {
        EvalValue::Integer(n) => *n,
        _ => {
            return Err(ReaperError::InvalidPolicy {
                reason: "time::format_rfc3339() requires an integer argument (nanoseconds)"
                    .to_string(),
            })
        }
    };

    use chrono::DateTime;
    let dt = DateTime::from_timestamp(nanos / 1_000_000_000, (nanos % 1_000_000_000) as u32)
        .ok_or_else(|| ReaperError::InvalidPolicy {
            reason: format!("Invalid timestamp: {}", nanos),
        })?;

    Ok(EvalValue::String(dt.to_rfc3339()))
}

/// time::add_ns(timestamp_ns, duration_ns) - Add duration to timestamp
#[inline]
pub fn add_ns(timestamp: &EvalValue, duration: &EvalValue) -> Result<EvalValue, ReaperError> {
    let ts = match timestamp {
        EvalValue::Integer(n) => *n,
        _ => {
            return Err(ReaperError::InvalidPolicy {
                reason: "time::add_ns() first argument must be an integer (nanoseconds)"
                    .to_string(),
            })
        }
    };
    let dur =
        match duration {
            EvalValue::Integer(n) => *n,
            _ => return Err(ReaperError::InvalidPolicy {
                reason:
                    "time::add_ns() second argument must be an integer (duration in nanoseconds)"
                        .to_string(),
            }),
        };

    Ok(EvalValue::Integer(ts.saturating_add(dur)))
}

/// time::subtract_ns(timestamp_ns, duration_ns) - Subtract duration from timestamp
#[inline]
pub fn subtract_ns(timestamp: &EvalValue, duration: &EvalValue) -> Result<EvalValue, ReaperError> {
    let ts = match timestamp {
        EvalValue::Integer(n) => *n,
        _ => {
            return Err(ReaperError::InvalidPolicy {
                reason: "time::subtract_ns() first argument must be an integer (nanoseconds)"
                    .to_string(),
            })
        }
    };
    let dur = match duration {
        EvalValue::Integer(n) => *n,
        _ => return Err(ReaperError::InvalidPolicy {
            reason:
                "time::subtract_ns() second argument must be an integer (duration in nanoseconds)"
                    .to_string(),
        }),
    };

    Ok(EvalValue::Integer(ts.saturating_sub(dur)))
}

/// time::is_before(t1, t2) - Check if t1 is before t2
#[inline]
pub fn is_before(t1: &EvalValue, t2: &EvalValue) -> Result<EvalValue, ReaperError> {
    let ts1 = match t1 {
        EvalValue::Integer(n) => *n,
        _ => {
            return Err(ReaperError::InvalidPolicy {
                reason: "time::is_before() arguments must be integers (timestamps)".to_string(),
            })
        }
    };
    let ts2 = match t2 {
        EvalValue::Integer(n) => *n,
        _ => {
            return Err(ReaperError::InvalidPolicy {
                reason: "time::is_before() arguments must be integers (timestamps)".to_string(),
            })
        }
    };

    Ok(EvalValue::Boolean(ts1 < ts2))
}

/// time::is_after(t1, t2) - Check if t1 is after t2
#[inline]
pub fn is_after(t1: &EvalValue, t2: &EvalValue) -> Result<EvalValue, ReaperError> {
    let ts1 = match t1 {
        EvalValue::Integer(n) => *n,
        _ => {
            return Err(ReaperError::InvalidPolicy {
                reason: "time::is_after() arguments must be integers (timestamps)".to_string(),
            })
        }
    };
    let ts2 = match t2 {
        EvalValue::Integer(n) => *n,
        _ => {
            return Err(ReaperError::InvalidPolicy {
                reason: "time::is_after() arguments must be integers (timestamps)".to_string(),
            })
        }
    };

    Ok(EvalValue::Boolean(ts1 > ts2))
}

/// time::is_between(t, start, end) - Check if t is between start and end (inclusive)
#[inline]
pub fn is_between(
    t: &EvalValue,
    start: &EvalValue,
    end: &EvalValue,
) -> Result<EvalValue, ReaperError> {
    let ts = match t {
        EvalValue::Integer(n) => *n,
        _ => {
            return Err(ReaperError::InvalidPolicy {
                reason: "time::is_between() arguments must be integers (timestamps)".to_string(),
            })
        }
    };
    let ts_start = match start {
        EvalValue::Integer(n) => *n,
        _ => {
            return Err(ReaperError::InvalidPolicy {
                reason: "time::is_between() arguments must be integers (timestamps)".to_string(),
            })
        }
    };
    let ts_end = match end {
        EvalValue::Integer(n) => *n,
        _ => {
            return Err(ReaperError::InvalidPolicy {
                reason: "time::is_between() arguments must be integers (timestamps)".to_string(),
            })
        }
    };

    Ok(EvalValue::Boolean(ts >= ts_start && ts <= ts_end))
}
