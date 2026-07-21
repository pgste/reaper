//! Compiled `input`-document access types (R4-01 Phase B.1; design in
//! docs/development/COMPILED_INPUT_DESIGN.md §3.1-§3.2).
//!
//! The compiled path navigates the request's raw `serde_json::Value`
//! document directly: paths are pre-parsed at compile time (no string
//! splitting at eval), and input values never touch the interner (document
//! keys/values are request-scoped, not policy text).

use serde::{Deserialize, Serialize};

/// One pre-parsed step of an `input` path.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum InputPathSeg {
    /// `.field` — object key lookup.
    Key(String),
}

/// A pre-parsed `input.<dotted.path>` (B.1 scope: dotted keys only —
/// bracket indexes and wildcards keep their AST fallback until B.2, whose
/// iteration source owns wildcard semantics).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InputPath {
    /// The steps after `input`, in order.
    pub segs: Vec<InputPathSeg>,
}

impl InputPath {
    /// Parse a dotted attribute string (`"request.object.metadata"`) into
    /// pre-split segments. Called at compile time only.
    pub fn from_dotted(attribute: &str) -> Self {
        Self {
            segs: attribute
                .split('.')
                .map(|p| InputPathSeg::Key(p.to_string()))
                .collect(),
        }
    }

    /// Walk the raw document. `None` ⇔ the AST's `Null` outcome: missing
    /// key, non-object intermediate, or no traversal possible — mirroring
    /// `navigate_eval_path` (Object-or-Null at every step, total, no error
    /// path).
    pub fn resolve<'a>(&self, doc: &'a serde_json::Value) -> Option<&'a serde_json::Value> {
        let mut current = doc;
        for seg in &self.segs {
            match seg {
                InputPathSeg::Key(k) => {
                    current = current.as_object()?.get(k)?;
                    // navigate_eval_path treats an explicit JSON null mid-path
                    // as terminal Null; `as_object()?` on Null does the same
                    // here on the next step, and a TERMINAL null is handled by
                    // the comparison (Null semantics), so map it to None only
                    // when traversal must continue — which the next loop
                    // iteration's `as_object()?` already does. Nothing extra
                    // needed.
                }
            }
        }
        Some(current)
    }
}

/// A scalar literal an `input` path is compared against. Kept raw (no
/// interning): input comparisons run over document values, not interned
/// policy strings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum InputLiteral {
    /// `== null` / `!= null` presence checks.
    Null,
    /// Boolean literal.
    Bool(bool),
    /// Integer literal.
    Int(i64),
    /// Float literal.
    Float(f64),
    /// String literal.
    Str(String),
}
