//! Reference-doc parse gate (round-3 Plan 04, Step 4).
//!
//! `docs/reference/reap-language.md` is the customer-facing `.reap` language
//! reference. Docs drift silently: an engine grammar change can leave every
//! example in the reference documenting syntax that no longer parses, and
//! nothing catches it — the reference had exactly this drift (v1 `package` /
//! `metadata {}` / `default deny` / `rule X { allow when {} }` blocks that the
//! v2 grammar rejects).
//!
//! This gate makes that impossible: every fenced ```reap block in the reference
//! must parse under the CURRENT grammar via `ReaperPolicy::from_str`. A block
//! that is a complete policy (`policy NAME { ... }`) is parsed as-is; a block
//! that is a bare condition/expression fragment is wrapped in a minimal rule and
//! parsed, so condition snippets are held to the same standard. If the grammar
//! moves and the reference is not updated, this turns red — the reference can
//! never again document syntax that does not parse.
//!
//! Blocks the reference intends as NON-`.reap` (Rego, Cedar, JSON, shell) use a
//! different fence tag and are not extracted. To show a `.reap` snippet that is
//! deliberately NOT meant to parse (there are none today), tag it `reap-invalid`.

use policy_engine::reap::{ReapParser, ReaperPolicy};
use std::path::{Path, PathBuf};
use std::str::FromStr;

fn reference_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/reference/reap-language.md")
}

/// A fenced code block extracted from the markdown, with its info string and
/// 1-based starting line (for a legible failure message).
struct Block {
    tag: String,
    start_line: usize,
    body: String,
}

/// Extract every fenced code block (``` … ```), capturing its info-string tag.
fn fenced_blocks(md: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut lines = md.lines().enumerate();
    while let Some((idx, line)) = lines.next() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("```") {
            let tag = rest.trim().to_string();
            let start_line = idx + 1;
            let mut body = String::new();
            for (_, inner) in lines.by_ref() {
                if inner.trim_start().starts_with("```") {
                    break;
                }
                body.push_str(inner);
                body.push('\n');
            }
            blocks.push(Block {
                tag,
                start_line,
                body,
            });
        }
    }
    blocks
}

/// First non-comment, non-blank token of a `.reap` block — used to decide
/// whether the block is a complete policy or a bare condition fragment.
fn first_token(body: &str) -> String {
    for raw in body.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("//") || line.starts_with('#') {
            continue;
        }
        return line.split_whitespace().next().unwrap_or("").to_string();
    }
    String::new()
}

/// Wrap a bare condition fragment in a minimal policy so it is parsed under the
/// exact grammar path a real rule condition takes.
fn wrap_fragment(fragment: &str) -> String {
    format!("policy doc_snippet {{ default: deny, rule doc_rule {{ allow if {{ {fragment} }} }} }}")
}

#[test]
fn every_reap_block_in_the_reference_parses() {
    let path = reference_path();
    let md = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));

    let reap_blocks: Vec<Block> = fenced_blocks(&md)
        .into_iter()
        .filter(|b| b.tag == "reap")
        .collect();

    assert!(
        !reap_blocks.is_empty(),
        "expected at least one ```reap block in the reference; found none — the \
         extractor or the doc changed shape"
    );

    let mut failures = Vec::new();
    for block in &reap_blocks {
        let token = first_token(&block.body);
        // Three top-level forms (language v3): a policy (optionally preceded
        // by imports — grammar-parsed directly, since `from_str` deliberately
        // rejects unresolved imports at LOAD, not at parse), a library file,
        // and bare condition fragments.
        let (kind, result) = match token.as_str() {
            "policy" => ("policy", ReaperPolicy::from_str(&block.body).map(|_| ())),
            "import" => (
                "importing policy",
                ReapParser::parse(&block.body).map(|_| ()),
            ),
            "library" => (
                "library",
                ReapParser::parse_library(&block.body).map(|_| ()),
            ),
            _ => (
                "fragment",
                ReaperPolicy::from_str(&wrap_fragment(block.body.trim())).map(|_| ()),
            ),
        };
        if let Err(e) = result {
            failures.push(format!(
                "  - {} block at reap-language.md:{} failed to parse: {e}",
                kind, block.start_line
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "the `.reap` language reference contains {} block(s) that do not parse \
         under the current grammar. Every ```reap example must be valid — fix the \
         doc (or retag a deliberately-non-.reap block):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Self-test: the extractor and wrapper actually exercise the parser — a known
/// bad fragment must fail, so a future change that neuters the check (e.g. the
/// wrapper producing always-parseable garbage) is caught.
#[test]
fn wrapper_actually_parses_fragments() {
    // Valid fragment parses.
    assert!(ReaperPolicy::from_str(&wrap_fragment(r#"user.role == "admin""#)).is_ok());
    // Invalid fragment (v1 implicit-AND, two bare comparisons) must fail — proof
    // the gate has teeth.
    assert!(ReaperPolicy::from_str(&wrap_fragment(
        "user.role == \"admin\"\nuser.active == true"
    ))
    .is_err());
}
