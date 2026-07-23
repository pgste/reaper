// ! Reaper Policy Language Parser and Compiler
//!
//! Parses .reap files into AST and compiles to ReaperDSLEvaluator for sub-microsecond evaluation.

mod ast;
mod ast_evaluator;
mod bundle;
mod compiler;
mod functions;
mod limits;
mod mixed_evaluator;
mod parser;
mod yaml_parser;

pub use ast::{
    AssignmentValue, ComparisonLeft, ComparisonRight, Condition as ReapCondition, Decision, Entity,
    EntityAttr, Expr, FuncDef, ImportDecl, Index, Operator, Policy, Rule as ReapRule,
    Value as ReapValue, VarAttr,
};
pub use ast_evaluator::{CheckResult, ReapAstEvaluator, Violation};
pub use bundle::{
    stable_policy_id, BundleFormat, PackageMetadata, PolicyBundle, PolicyEntry, PolicyPackage,
    PrecompilationHints,
};
pub use compiler::compile_policy;
pub use limits::{
    configured_max_nesting_depth, enforce_policy_depth, enforce_source_nesting,
    DEFAULT_MAX_NESTING_DEPTH,
};
pub use mixed_evaluator::{MixedReapEvaluator, PerRuleBuild};
pub use parser::ReapParser;
pub use yaml_parser::YamlPolicy;

use crate::data::DataStore;
use crate::evaluators::reaper_dsl::ReaperDSLEvaluator;
use reaper_core::ReaperError;
use std::fs;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

/// Main entry point for loading .reap policies
#[derive(Clone, Debug)]
pub struct ReaperPolicy {
    ast: Policy,
}

impl ReaperPolicy {
    /// Load a .reap file from disk. `import "path" as ns` declarations
    /// resolve HERE, at load time, against the policy file's directory: each
    /// imported library is read, parsed, namespaced under its alias, and its
    /// functions merged into the policy AST — evaluation (and the bundle
    /// format) never touches the filesystem again.
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, ReaperError> {
        let path = path.as_ref();
        let content = fs::read_to_string(path).map_err(|e| ReaperError::InvalidPolicy {
            reason: format!("Failed to read policy file: {}", e),
        })?;
        let mut policy = Self::parse_source(&content)?;
        if !policy.ast.imports.is_empty() {
            let base_dir = path.parent().unwrap_or_else(|| Path::new("."));
            resolve_imports(&mut policy.ast, base_dir)?;
        }
        Ok(policy)
    }

    /// Parse `.reap` source without restricting imports (shared by
    /// [`FromStr`] — which rejects unresolved imports — and by
    /// [`Self::from_file`], which resolves them against the file's
    /// directory). Stamps `language_version: "3"` when the policy uses v3
    /// constructs (`func`/`import`) and declares no version, so every
    /// downstream artifact (bundles included) carries the marker that makes
    /// older engines fail closed.
    fn parse_source(s: &str) -> Result<Self, ReaperError> {
        let mut ast = ReapParser::parse(s)?;
        let uses_v3 = !ast.functions.is_empty() || !ast.imports.is_empty();
        if uses_v3 && !ast.metadata.contains_key("language_version") {
            ast.metadata.insert(
                "language_version".to_string(),
                FUNC_IMPORT_MIN_LANGUAGE_VERSION.to_string(),
            );
        }
        let policy = Self { ast };
        policy.check_language_version()?;
        Ok(policy)
    }

    /// Parse a policy from YAML string
    pub fn from_yaml_str(input: &str) -> Result<Self, ReaperError> {
        let yaml_policy = YamlPolicy::from_yaml(input)?;
        let ast = yaml_policy.to_ast()?;
        Ok(Self { ast })
    }

    /// Load a YAML policy file from disk
    pub fn from_yaml_file<P: AsRef<Path>>(path: P) -> Result<Self, ReaperError> {
        let content =
            fs::read_to_string(path.as_ref()).map_err(|e| ReaperError::InvalidPolicy {
                reason: format!("Failed to read YAML policy file: {}", e),
            })?;
        Self::from_yaml_str(&content)
    }

    /// Parse a policy from JSON string
    pub fn from_json_str(input: &str) -> Result<Self, ReaperError> {
        let yaml_policy = YamlPolicy::from_json(input)?;
        let ast = yaml_policy.to_ast()?;
        Ok(Self { ast })
    }

    /// Load a JSON policy file from disk
    pub fn from_json_file<P: AsRef<Path>>(path: P) -> Result<Self, ReaperError> {
        let content =
            fs::read_to_string(path.as_ref()).map_err(|e| ReaperError::InvalidPolicy {
                reason: format!("Failed to read JSON policy file: {}", e),
            })?;
        Self::from_json_str(&content)
    }

    /// Auto-detect format and load from file based on extension
    /// Supports .reap, .yaml, .yml, .json
    pub fn from_file_auto<P: AsRef<Path>>(path: P) -> Result<Self, ReaperError> {
        let path_ref = path.as_ref();
        let extension = path_ref
            .extension()
            .and_then(|e| e.to_str())
            .ok_or_else(|| ReaperError::InvalidPolicy {
                reason: "File has no extension. Use .reap, .yaml, .yml, or .json".to_string(),
            })?;

        match extension.to_lowercase().as_str() {
            "reap" => Self::from_file(path_ref),
            "yaml" | "yml" => Self::from_yaml_file(path_ref),
            "json" => Self::from_json_file(path_ref),
            _ => Err(ReaperError::InvalidPolicy {
                reason: format!(
                    "Unsupported file extension '.{}'. Use .reap, .yaml, .yml, or .json",
                    extension
                ),
            }),
        }
    }

    /// Build a ReaperDSLEvaluator from this policy (compiled, optimized)
    pub fn build(self, store: Arc<DataStore>) -> Result<ReaperDSLEvaluator, ReaperError> {
        compiler::compile_policy(self.ast, store)
    }

    /// Build a ReapAstEvaluator from this policy (direct AST evaluation, supports all features)
    ///
    /// Use this for policies that use advanced features like comprehensions,
    /// variable assignments, or other features not yet supported by the compiler.
    pub fn build_ast_evaluator(self, store: Arc<DataStore>) -> ReapAstEvaluator {
        ReapAstEvaluator::new(store, self.ast)
    }

    /// Build the PREFERRED evaluator: the compiled `ReaperDSLEvaluator` when
    /// this policy compiles whole; otherwise a per-RULE mixed evaluator that
    /// keeps every compilable rule on the compiled path and interprets only
    /// the rules that need it (R4-01 Phase A.2); a whole-policy
    /// `ReapAstEvaluator` only when no rule compiles at all.
    ///
    /// The compiled path is faster; the AST path supports every feature. All
    /// three shapes are required to produce identical decisions for any
    /// policy (pinned by the compiled-vs-AST and mixed-mode differentials),
    /// so falling back never changes an authorization outcome — it only
    /// trades speed for coverage on constructs the compiler doesn't yet
    /// handle. This is the entry point production code should use unless it
    /// specifically needs one implementation.
    pub fn build_preferred(
        self,
        store: Arc<DataStore>,
    ) -> Result<Box<dyn crate::evaluators::PolicyEvaluator>, ReaperError> {
        match compiler::compile_policy(self.clone().ast, store.clone()) {
            Ok(compiled) => Ok(Box::new(compiled)),
            Err(compile_err) => {
                match mixed_evaluator::MixedReapEvaluator::build(self.ast, store)? {
                    PerRuleBuild::Mixed(mixed) => {
                        let (compiled_rules, ast_rules) = mixed.rule_modes();
                        tracing::info!(
                            %compile_err,
                            compiled_rules,
                            ast_rules,
                            ast_rule_names = %mixed.ast_rule_names().join(","),
                            "policy did not compile whole; serving mixed-mode \
                             (per-rule compiled/AST fallback)"
                        );
                        Ok(Box::new(mixed))
                    }
                    PerRuleBuild::AllAst(ast) => {
                        tracing::debug!(
                            %compile_err,
                            "no rule compiled; falling back to whole-policy AST evaluator"
                        );
                        Ok(Box::new(ast))
                    }
                }
            }
        }
    }

    /// Get the policy name
    pub fn name(&self) -> &str {
        &self.ast.name
    }

    /// Get the policy version (if any)
    pub fn version(&self) -> Option<&str> {
        self.ast.metadata.get("version").map(|s| s.as_str())
    }

    /// Get the policy package name (defaults to "default" if not specified)
    pub fn package(&self) -> &str {
        self.ast
            .metadata
            .get("package")
            .map(|s| s.as_str())
            .unwrap_or("default")
    }

    /// Get the policy description (if any)
    pub fn description(&self) -> Option<&str> {
        self.ast.metadata.get("description").map(|s| s.as_str())
    }

    /// Get all policy metadata as a HashMap
    pub fn metadata(&self) -> &std::collections::HashMap<String, String> {
        &self.ast.metadata
    }

    /// Compile to a binary bundle for fast loading
    pub fn compile_to_bundle(&self) -> Result<Vec<u8>, ReaperError> {
        bundle::compile_to_bundle(&self.ast)
    }

    /// Load from a binary bundle
    pub fn from_bundle(
        bytes: &[u8],
        store: Arc<DataStore>,
    ) -> Result<ReaperDSLEvaluator, ReaperError> {
        let policy = bundle::load_from_bundle(bytes)?;
        compiler::compile_policy(policy, store)
    }
}

/// The `.reap` DSL language version this engine implements.
///
/// A policy may declare its target version with a `language_version: "N"`
/// metadata field. A policy declaring a NEWER version than this is rejected
/// (fail-closed), the same posture the bundle wire format takes one layer down
/// (`reap/bundle.rs`) — an old engine must never silently misinterpret a policy
/// written against a newer language. A policy that declares no version is
/// treated as this current (implicit) version. Bump this only alongside a
/// documented, frozen-corpus-gated language change (see
/// `docs/reference/DSL_COMPATIBILITY.md`).
///
/// Version history: 2 → 3 (R4-01 Phase C): helper predicates (`func`) and
/// load-time imports (`import "path" as ns`). Additive for v2 policies (no
/// existing decision changes — frozen corpus stays green), but policies USING
/// the new constructs are stamped/required `language_version: "3"` so v2
/// engines reject their artifacts instead of silently dropping the function
/// definitions (the bundle wire format is not self-describing).
pub const CURRENT_LANGUAGE_VERSION: u32 = 3;

/// The minimum language version a policy that uses `func` or `import` must
/// declare. `parse_source` stamps it automatically when the author declares
/// none; an explicitly OLDER declaration is a hard error (`check_language_version`).
pub const FUNC_IMPORT_MIN_LANGUAGE_VERSION: u32 = 3;

impl FromStr for ReaperPolicy {
    type Err = ReaperError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let policy = Self::parse_source(s)?;
        if let Some(first) = policy.ast.imports.first() {
            // No base directory to resolve against — string-parsed policies
            // (API deploys, inline sources) cannot import. File loads and
            // pre-resolved bundles can.
            return Err(ReaperError::InvalidPolicy {
                reason: format!(
                    "policy imports \"{}\" but was parsed from a string; imports \
                     resolve at load time, so load the policy from a file \
                     (reaper-cli / from_file) or deploy a compiled bundle with \
                     the imports already resolved",
                    first.path
                ),
            });
        }
        Ok(policy)
    }
}

/// Resolve every `import "path" as alias` in `policy` against `base_dir`:
/// read + parse each library, rewrite its internal (library-local) calls to
/// the alias namespace, tag its functions with the alias, and merge them into
/// `policy.functions`. Re-validates the merged whole (call-graph DAG across
/// policy + libraries, depth accounting, strict call resolution).
fn resolve_imports(policy: &mut Policy, base_dir: &Path) -> Result<(), ReaperError> {
    let mut seen_aliases = std::collections::HashSet::new();
    let imports = policy.imports.clone();
    for import in &imports {
        if functions::BUILTIN_NAMESPACES.contains(&import.alias.as_str()) {
            return Err(ReaperError::InvalidPolicy {
                reason: format!(
                    "import alias '{}' collides with a builtin namespace",
                    import.alias
                ),
            });
        }
        if !seen_aliases.insert(import.alias.clone()) {
            return Err(ReaperError::InvalidPolicy {
                reason: format!("duplicate import alias '{}'", import.alias),
            });
        }

        // Load-time path hygiene: relative, no parent traversal, .reap only.
        let rel = Path::new(&import.path);
        if rel.is_absolute() {
            return Err(ReaperError::InvalidPolicy {
                reason: format!(
                    "import path \"{}\" is absolute; imports must be relative to \
                     the importing file",
                    import.path
                ),
            });
        }
        if rel
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(ReaperError::InvalidPolicy {
                reason: format!(
                    "import path \"{}\" contains '..'; imports may not traverse \
                     above the importing file's directory",
                    import.path
                ),
            });
        }
        if rel.extension().and_then(|e| e.to_str()) != Some("reap") {
            return Err(ReaperError::InvalidPolicy {
                reason: format!(
                    "import path \"{}\" must reference a .reap library file",
                    import.path
                ),
            });
        }

        let full = base_dir.join(rel);
        let content = fs::read_to_string(&full).map_err(|e| ReaperError::InvalidPolicy {
            reason: format!(
                "failed to read imported library \"{}\": {}",
                full.display(),
                e
            ),
        })?;
        let (_lib_name, mut lib_funcs) = ReapParser::parse_library(&content)?;

        // Rewrite library-internal calls (`helper(x)` referring to a sibling
        // in the same library) to the alias namespace, then tag the
        // functions themselves. After this, the merged set has no ambiguous
        // un-namespaced references into the library.
        let local_names: std::collections::HashSet<String> =
            lib_funcs.iter().map(|f| f.name.clone()).collect();
        for f in &mut lib_funcs {
            namespace_local_calls(&mut f.body, &local_names, &import.alias);
            f.namespace = Some(import.alias.clone());
        }
        policy.functions.extend(lib_funcs);
    }

    // The merged set must satisfy everything the parse-time check deferred:
    // every aliased call resolves (arity included), the cross-library call
    // graph is a DAG, and inline-expanded depth stays within the cap.
    functions::verify_resolved_calls(policy, limits::configured_max_nesting_depth())
}

/// Rewrite un-namespaced calls to library-local functions into the import
/// alias namespace, recursively through the condition tree.
fn namespace_local_calls(
    cond: &mut ast::Condition,
    local_names: &std::collections::HashSet<String>,
    alias: &str,
) {
    fn rewrite_expr(expr: &mut Expr, local_names: &std::collections::HashSet<String>, alias: &str) {
        match expr {
            Expr::FunctionCall {
                namespace,
                function,
                args,
            } => {
                if namespace.is_none() && local_names.contains(function.as_str()) {
                    *namespace = Some(alias.to_string());
                }
                for a in args {
                    rewrite_expr(a, local_names, alias);
                }
            }
            Expr::MethodCall { receiver, args, .. } => {
                rewrite_expr(receiver, local_names, alias);
                for a in args {
                    rewrite_expr(a, local_names, alias);
                }
            }
            Expr::Literal(_)
            | Expr::Variable(_)
            | Expr::AttributeAccess { .. }
            | Expr::IndexedAccess { .. } => {}
        }
    }
    fn rewrite_assignment(
        value: &mut AssignmentValue,
        local_names: &std::collections::HashSet<String>,
        alias: &str,
    ) {
        match value {
            AssignmentValue::Expr(e) => rewrite_expr(e, local_names, alias),
            AssignmentValue::Comparison { left, right, .. } => {
                if let ComparisonLeft::Expr(e) = left {
                    rewrite_expr(e, local_names, alias);
                }
                if let ComparisonRight::Expr(e) = right {
                    rewrite_expr(e, local_names, alias);
                }
            }
            AssignmentValue::Comprehension(comp) => {
                let (outputs, filters) = match comp {
                    ast::Comprehension::Set {
                        output, filters, ..
                    }
                    | ast::Comprehension::Array {
                        output, filters, ..
                    } => (vec![output.as_mut()], filters),
                    ast::Comprehension::Object {
                        key,
                        value,
                        filters,
                        ..
                    } => (vec![key.as_mut(), value.as_mut()], filters),
                };
                for o in outputs {
                    rewrite_expr(o, local_names, alias);
                }
                for f in filters {
                    namespace_local_calls(f, local_names, alias);
                }
            }
            AssignmentValue::EntityAttr(_)
            | AssignmentValue::Value(_)
            | AssignmentValue::Variable(_) => {}
        }
    }
    match cond {
        ast::Condition::Expr(e) => rewrite_expr(e, local_names, alias),
        ast::Condition::Comparison { left, right, .. } => {
            if let ComparisonLeft::Expr(e) = left {
                rewrite_expr(e, local_names, alias);
            }
            if let ComparisonRight::Expr(e) = right {
                rewrite_expr(e, local_names, alias);
            }
        }
        ast::Condition::Assignment { value, .. } => rewrite_assignment(value, local_names, alias),
        ast::Condition::And(cs) | ast::Condition::Or(cs) => {
            for c in cs {
                namespace_local_calls(c, local_names, alias);
            }
        }
        ast::Condition::Not(inner) => namespace_local_calls(inner, local_names, alias),
        ast::Condition::True | ast::Condition::False => {}
    }
}

impl ReaperPolicy {
    /// The DSL language version this policy targets — its declared
    /// `language_version`, or [`CURRENT_LANGUAGE_VERSION`] if it declares none.
    pub fn language_version(&self) -> u32 {
        self.ast
            .metadata
            .get("language_version")
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(CURRENT_LANGUAGE_VERSION)
    }

    /// Fail closed if the policy declares a language version this engine does
    /// not implement (newer than [`CURRENT_LANGUAGE_VERSION`]), or a malformed
    /// one. Never down-levels or best-effort parses a newer policy.
    ///
    /// Also the forward direction: a policy that USES v3 constructs
    /// (`func`/`import`) while explicitly declaring an older version is
    /// rejected — the declaration is what makes older engines refuse the
    /// artifact, so it must not understate the constructs in use.
    pub(crate) fn check_language_version(&self) -> Result<(), ReaperError> {
        if let Some(raw) = self.ast.metadata.get("language_version") {
            let got = raw.parse::<u32>().map_err(|_| ReaperError::InvalidPolicy {
                reason: format!(
                    "malformed language_version {raw:?}: expected an integer like \"3\""
                ),
            })?;
            if got > CURRENT_LANGUAGE_VERSION {
                return Err(ReaperError::LanguageVersionUnsupported {
                    got,
                    supported: CURRENT_LANGUAGE_VERSION,
                });
            }
            let uses_v3 = !self.ast.functions.is_empty() || !self.ast.imports.is_empty();
            if uses_v3 && got < FUNC_IMPORT_MIN_LANGUAGE_VERSION {
                return Err(ReaperError::InvalidPolicy {
                    reason: format!(
                        "policy declares language_version \"{got}\" but uses func/import, \
                         which require language_version \
                         \"{FUNC_IMPORT_MIN_LANGUAGE_VERSION}\" — update the declaration \
                         (older engines must reject this policy rather than misread it)"
                    ),
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_policy() {
        let policy_text = r#"
            policy test_policy {
                default: deny,
                rule admin { allow if user.role == "admin" }
            }
        "#;

        let policy = ReaperPolicy::from_str(policy_text).unwrap();
        assert_eq!(policy.name(), "test_policy");
    }

    #[test]
    fn test_parse_with_metadata() {
        let policy_text = r#"
            policy test {
                version: "1.0.0",
                description: "Test policy",
                default: allow,
                rule test { deny if user.suspended == true }
            }
        "#;

        let policy = ReaperPolicy::from_str(policy_text).unwrap();
        assert_eq!(policy.version(), Some("1.0.0"));
    }

    // --- DSL language versioning (round-3 Plan 04, Step 2) ---

    #[test]
    fn headerless_policy_is_current_language_version() {
        let policy = ReaperPolicy::from_str(
            r#"policy p { default: deny, rule r { allow if user.role == "admin" } }"#,
        )
        .expect("headerless policy must still parse (back-compat)");
        assert_eq!(policy.language_version(), CURRENT_LANGUAGE_VERSION);
    }

    #[test]
    fn declared_current_language_version_parses() {
        let src = format!(
            r#"policy p {{ language_version: "{CURRENT_LANGUAGE_VERSION}", default: deny,
               rule r {{ allow if user.role == "admin" }} }}"#
        );
        let policy = ReaperPolicy::from_str(&src).expect("current version must parse");
        assert_eq!(policy.language_version(), CURRENT_LANGUAGE_VERSION);
    }

    #[test]
    fn newer_language_version_fails_closed() {
        let src = r#"policy p { language_version: "999", default: deny,
                     rule r { allow if user.role == "admin" } }"#;
        match ReaperPolicy::from_str(src) {
            Err(ReaperError::LanguageVersionUnsupported { got, supported }) => {
                assert_eq!(got, 999);
                assert_eq!(supported, CURRENT_LANGUAGE_VERSION);
            }
            Ok(_) => {
                panic!("a policy targeting a newer language must be rejected, not down-levelled")
            }
            Err(other) => panic!("expected LanguageVersionUnsupported, got {other:?}"),
        }
    }

    #[test]
    fn malformed_language_version_rejected() {
        let src = r#"policy p { language_version: "two", default: deny,
                     rule r { allow if user.role == "admin" } }"#;
        assert!(matches!(
            ReaperPolicy::from_str(src),
            Err(ReaperError::InvalidPolicy { .. })
        ));
    }
}
