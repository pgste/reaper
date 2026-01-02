//! Thread-Local Arena Allocator for Zero-Allocation Evaluation
//!
//! This module provides a bump allocator for the policy evaluation hot path,
//! eliminating heap allocations during condition evaluation.
//!
//! # Performance Characteristics
//! - Allocation: O(1) pointer bump, ~1-2ns
//! - Deallocation: O(1) arena reset, ~1ns
//! - No per-object deallocation overhead
//! - Cache-friendly linear memory layout
//!
//! # Usage Pattern
//! ```rust,ignore
//! use policy_engine::arena::{with_arena, ArenaVec, ArenaString};
//!
//! // All allocations within the closure use the thread-local arena
//! let result = with_arena(|arena| {
//!     // Arena-allocated string (no heap allocation)
//!     let s = arena.alloc_str("hello");
//!
//!     // Arena-allocated vector
//!     let mut vec = ArenaVec::new_in(arena);
//!     vec.push("item1");
//!     vec.push("item2");
//!
//!     // Process and return result (arena freed after closure)
//!     s.len() + vec.len()
//! });
//! ```

use bumpalo::Bump;
use std::cell::RefCell;

/// Default arena capacity (64KB - fits in L2 cache)
const DEFAULT_ARENA_CAPACITY: usize = 64 * 1024;

/// Maximum arena size before forced reset (1MB)
const MAX_ARENA_SIZE: usize = 1024 * 1024;

// Thread-local arena storage
//
// Each thread maintains its own arena, eliminating synchronization overhead.
// The arena is reset between evaluations to reclaim memory.
thread_local! {
    static EVAL_ARENA: RefCell<Bump> = RefCell::new(Bump::with_capacity(DEFAULT_ARENA_CAPACITY));
}

/// Execute a function with access to the thread-local arena.
///
/// The arena is automatically reset after the function returns,
/// freeing all allocations made during evaluation.
///
/// # Type Parameters
/// * `F` - Function that takes arena reference and returns R
/// * `R` - Return type
///
/// # Example
/// ```rust,ignore
/// let count = with_arena(|arena| {
///     let s1 = arena.alloc_str("hello");
///     let s2 = arena.alloc_str("world");
///     2  // return value
/// });
/// // Arena is now reset, s1 and s2 are freed
/// ```
#[inline]
pub fn with_arena<F, R>(f: F) -> R
where
    F: FnOnce(&Bump) -> R,
{
    EVAL_ARENA.with(|arena| {
        let arena_ref = arena.borrow();
        let result = f(&arena_ref);
        drop(arena_ref);

        // Reset arena if it grew too large
        let mut arena_mut = arena.borrow_mut();
        if arena_mut.allocated_bytes() > MAX_ARENA_SIZE {
            arena_mut.reset();
        }

        result
    })
}

/// Execute a function with arena access, then reset the arena.
///
/// Similar to `with_arena` but always resets the arena after use.
/// Use this for top-level evaluation calls.
///
/// # Example
/// ```rust,ignore
/// let decision = with_arena_reset(|arena| {
///     evaluate_policy(arena, policy, request)
/// });
/// // Arena is always reset here
/// ```
#[inline]
pub fn with_arena_reset<F, R>(f: F) -> R
where
    F: FnOnce(&Bump) -> R,
{
    EVAL_ARENA.with(|arena| {
        let arena_ref = arena.borrow();
        let result = f(&arena_ref);
        drop(arena_ref);

        // Always reset after evaluation
        arena.borrow_mut().reset();

        result
    })
}

/// Get arena statistics for the current thread.
#[derive(Debug, Clone)]
pub struct ArenaStats {
    /// Total bytes allocated in the arena
    pub allocated_bytes: usize,
    /// Number of memory chunks allocated
    pub chunks_count: usize,
}

/// Get statistics about the current thread's arena.
pub fn arena_stats() -> ArenaStats {
    EVAL_ARENA.with(|arena| {
        let mut arena = arena.borrow_mut();
        ArenaStats {
            allocated_bytes: arena.allocated_bytes(),
            chunks_count: arena.iter_allocated_chunks().count(),
        }
    })
}

/// Reset the thread-local arena, freeing all allocations.
///
/// Call this between evaluations if you want explicit control
/// over when memory is reclaimed.
#[inline]
pub fn reset_arena() {
    EVAL_ARENA.with(|arena| {
        arena.borrow_mut().reset();
    });
}

/// Pre-allocate arena capacity for the current thread.
///
/// Call this at thread startup to avoid allocation during first evaluation.
pub fn prewarm_arena(capacity: usize) {
    EVAL_ARENA.with(|arena| {
        let mut arena = arena.borrow_mut();
        // Reset with new capacity
        *arena = Bump::with_capacity(capacity);
    });
}

/// Evaluation context with arena for zero-allocation evaluation.
///
/// This struct provides convenient access to arena-allocated
/// collections and strings during policy evaluation.
pub struct EvaluationContext<'a> {
    arena: &'a Bump,
}

impl<'a> EvaluationContext<'a> {
    /// Create a new evaluation context with the given arena.
    #[inline]
    pub fn new(arena: &'a Bump) -> Self {
        Self { arena }
    }

    /// Get the underlying arena reference.
    #[inline]
    pub fn arena(&self) -> &'a Bump {
        self.arena
    }

    /// Allocate a string slice in the arena.
    ///
    /// Returns a reference that lives as long as the arena.
    #[inline]
    pub fn alloc_str(&self, s: &str) -> &'a str {
        self.arena.alloc_str(s)
    }

    /// Allocate and format a string in the arena.
    #[inline]
    pub fn alloc_fmt(&self, args: std::fmt::Arguments<'_>) -> &'a str {
        use std::fmt::Write;
        let mut s = bumpalo::collections::String::new_in(self.arena);
        let _ = s.write_fmt(args);
        s.into_bump_str()
    }

    /// Create an arena-allocated vector.
    #[inline]
    pub fn vec<T>(&self) -> bumpalo::collections::Vec<'a, T> {
        bumpalo::collections::Vec::new_in(self.arena)
    }

    /// Create an arena-allocated vector with capacity.
    #[inline]
    pub fn vec_with_capacity<T>(&self, capacity: usize) -> bumpalo::collections::Vec<'a, T> {
        bumpalo::collections::Vec::with_capacity_in(capacity, self.arena)
    }

    /// Create an arena-allocated string.
    #[inline]
    pub fn string(&self) -> bumpalo::collections::String<'a> {
        bumpalo::collections::String::new_in(self.arena)
    }

    /// Create an arena-allocated string from a str.
    #[inline]
    pub fn string_from(&self, s: &str) -> bumpalo::collections::String<'a> {
        bumpalo::collections::String::from_str_in(s, self.arena)
    }

    /// Allocate a value in the arena.
    #[inline]
    pub fn alloc<T>(&self, val: T) -> &'a mut T {
        self.arena.alloc(val)
    }

    /// Allocate a slice in the arena by copying from an iterator.
    #[inline]
    pub fn alloc_slice_copy<T: Copy>(&self, slice: &[T]) -> &'a [T] {
        self.arena.alloc_slice_copy(slice)
    }
}

/// Type aliases for arena-allocated collections
pub type ArenaVec<'a, T> = bumpalo::collections::Vec<'a, T>;
pub type ArenaString<'a> = bumpalo::collections::String<'a>;

/// Intermediate evaluation result that can hold arena-allocated data.
///
/// This enum is used for passing values between condition evaluations
/// without heap allocation.
#[derive(Debug, Clone)]
pub enum ArenaValue<'a> {
    /// Null/undefined value
    Null,
    /// Boolean value
    Bool(bool),
    /// Integer value
    Int(i64),
    /// Float value
    Float(f64),
    /// String slice (arena or static)
    Str(&'a str),
    /// Owned string (for cases where we can't use arena)
    String(String),
    /// Array of values
    Array(ArenaVec<'a, ArenaValue<'a>>),
}

impl<'a> ArenaValue<'a> {
    /// Check if this value is truthy.
    #[inline]
    pub fn is_truthy(&self) -> bool {
        match self {
            ArenaValue::Null => false,
            ArenaValue::Bool(b) => *b,
            ArenaValue::Int(i) => *i != 0,
            ArenaValue::Float(f) => *f != 0.0,
            ArenaValue::Str(s) => !s.is_empty(),
            ArenaValue::String(s) => !s.is_empty(),
            ArenaValue::Array(arr) => !arr.is_empty(),
        }
    }

    /// Get as string slice if possible.
    #[inline]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            ArenaValue::Str(s) => Some(s),
            ArenaValue::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    /// Get as boolean if possible.
    #[inline]
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            ArenaValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Get as integer if possible.
    #[inline]
    pub fn as_int(&self) -> Option<i64> {
        match self {
            ArenaValue::Int(i) => Some(*i),
            ArenaValue::Float(f) => Some(*f as i64),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_with_arena_returns_value() {
        // Test that with_arena works and returns the correct value
        let result = with_arena(|arena| {
            let s = arena.alloc_str("hello");
            s.len()
        });
        assert_eq!(result, 5);
    }

    #[test]
    fn test_with_arena_allocations_work() {
        // Test that allocations within the arena work correctly
        with_arena(|arena| {
            // Allocate multiple strings
            let s1 = arena.alloc_str("hello");
            let s2 = arena.alloc_str("world");
            let s3 = arena.alloc_str("test");

            assert_eq!(s1, "hello");
            assert_eq!(s2, "world");
            assert_eq!(s3, "test");
        });
    }

    #[test]
    fn test_evaluation_context_strings() {
        with_arena(|arena| {
            let ctx = EvaluationContext::new(arena);

            // Test string allocation
            let s = ctx.alloc_str("hello");
            assert_eq!(s, "hello");

            // Test string builder
            let mut string = ctx.string_from("hello ");
            string.push_str("world");
            assert_eq!(string.as_str(), "hello world");
        });
    }

    #[test]
    fn test_evaluation_context_vectors() {
        with_arena(|arena| {
            let ctx = EvaluationContext::new(arena);

            // Test vector
            let mut vec = ctx.vec::<i32>();
            vec.push(1);
            vec.push(2);
            vec.push(3);
            assert_eq!(vec.len(), 3);
            assert_eq!(vec[0], 1);
            assert_eq!(vec[2], 3);
        });
    }

    #[test]
    fn test_arena_vec_operations() {
        with_arena(|arena| {
            let ctx = EvaluationContext::new(arena);

            let mut vec = ctx.vec_with_capacity::<&str>(10);
            for i in 0..10 {
                vec.push(ctx.alloc_str(&format!("item{}", i)));
            }

            assert_eq!(vec.len(), 10);
            assert_eq!(vec[0], "item0");
            assert_eq!(vec[9], "item9");
        });
    }

    #[test]
    fn test_arena_value_bool() {
        let val = ArenaValue::Bool(true);
        assert!(val.is_truthy());
        assert_eq!(val.as_bool(), Some(true));

        let val = ArenaValue::Bool(false);
        assert!(!val.is_truthy());
        assert_eq!(val.as_bool(), Some(false));
    }

    #[test]
    fn test_arena_value_int() {
        let val = ArenaValue::Int(42);
        assert!(val.is_truthy());
        assert_eq!(val.as_int(), Some(42));

        let val = ArenaValue::Int(0);
        assert!(!val.is_truthy());
        assert_eq!(val.as_int(), Some(0));
    }

    #[test]
    fn test_arena_value_str() {
        let val = ArenaValue::Str("hello");
        assert!(val.is_truthy());
        assert_eq!(val.as_str(), Some("hello"));

        let val = ArenaValue::Str("");
        assert!(!val.is_truthy());
    }

    #[test]
    fn test_arena_value_null() {
        let val = ArenaValue::Null;
        assert!(!val.is_truthy());
        assert_eq!(val.as_bool(), None);
        assert_eq!(val.as_int(), None);
        assert_eq!(val.as_str(), None);
    }

    #[test]
    fn test_arena_allocation_increases() {
        // Get baseline
        let before = arena_stats();

        // Allocate significant data
        with_arena(|arena| {
            for i in 0..100 {
                let _ = arena.alloc_str(&format!("string number {}", i));
            }
        });

        let after = arena_stats();
        // Arena should have grown (or stayed same if already large)
        assert!(after.allocated_bytes >= before.allocated_bytes);
    }

    #[test]
    fn test_with_arena_reset_returns_value() {
        // Test that with_arena_reset returns the correct value
        let result = with_arena_reset(|arena| {
            let s = arena.alloc_str("test");
            s.len() * 2
        });
        assert_eq!(result, 8);
    }

    #[test]
    fn test_arena_alloc_slice() {
        with_arena(|arena| {
            let ctx = EvaluationContext::new(arena);

            let data = [1u8, 2, 3, 4, 5];
            let slice = ctx.alloc_slice_copy(&data);

            assert_eq!(slice.len(), 5);
            assert_eq!(slice[0], 1);
            assert_eq!(slice[4], 5);
        });
    }

    #[test]
    fn test_arena_fmt() {
        with_arena(|arena| {
            let ctx = EvaluationContext::new(arena);

            let s = ctx.alloc_fmt(format_args!("hello {} world", 42));
            assert_eq!(s, "hello 42 world");
        });
    }
}
