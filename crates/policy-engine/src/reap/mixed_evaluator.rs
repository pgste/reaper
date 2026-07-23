//! Mixed-mode policy evaluator (R4-01 Phase A.2): per-RULE compiled/AST
//! fallback.
//!
//! Before this, `Policy::build_preferred` was all-or-nothing: one construct
//! the compiler doesn't support anywhere in a policy sent the ENTIRE policy
//! to the AST interpreter — a silent 10-100× slowdown for every rule,
//! including the perfectly compilable ones. This evaluator splits the policy
//! at rule granularity: each rule is compiled when it can be, interpreted
//! when it can't, and the policy-level semantics are re-composed here.
//!
//! ## Semantics — identical to the whole-policy evaluators by construction
//!
//! The engine's contract (docs/reference/reap-language.md "Evaluation
//! Semantics") is deny-overrides, then first-allow-wins, then default:
//!
//! 1. every `deny` rule, in source order — first match ⇒ Deny, stop;
//! 2. every `allow` rule, in source order — first match ⇒ Allow, stop;
//! 3. no match ⇒ the policy's `default:`.
//!
//! Both the compiled evaluator and the AST evaluator implement exactly this
//! loop internally; this evaluator runs the SAME loop over single-rule
//! sub-evaluators, so rule ordering is the source order regardless of which
//! side of the compile line each rule landed on. Rule scope is already
//! per-rule (`variables` clears between rules in both evaluators), so
//! splitting rules across evaluators cannot leak state between them.
//!
//! A unit "matched" iff its named outcome carries a rule name — both
//! evaluators return `Some(name)` exactly when a rule matched and `None`
//! when their per-policy default answered (the sub-policy default is
//! therefore never surfaced by this wrapper; the parent default is applied
//! once, in step 3).
//!
//! ## Outward contract — mirrors the AST fallback it replaces
//!
//! Every policy this evaluator serves was previously served whole by
//! `ReapAstEvaluator` (that is the definition of "did not fully compile").
//! To make this a pure SPEED change with zero decision/semantics delta:
//!
//! - `evaluate_matched`/`evaluate_named` report `matched: true` always,
//!   exactly like the AST evaluator's deliberate always-decisive contract —
//!   set-level combination (`PolicyEngine::evaluate_set`) must not change
//!   for these policies under a performance feature.
//! - `resource_pruning()` stays at the trait default (`Unprunable`), the
//!   same bound the whole-AST policy had. (A compiled-rule union bound is a
//!   possible later slice; it must then union in the AST rules'
//!   unprunability anyway, which keeps the policy unprunable — so nothing
//!   is being left on the table today.)
//!
//! One knowing divergence, documented: the per-evaluation ReBAC traversal
//! budget resets at each sub-evaluator's entry, so a mixed policy gives each
//! RULE a fresh budget where the whole-policy evaluators share one budget
//! across all rules. The budget is an availability guard, not semantics — a
//! per-rule budget is strictly closer to "every rule fully evaluated", and a
//! policy near the shared-budget cliff was already in fail-closed territory.
//!
//! The mixed-vs-AST differential (`tests/mixed_mode_differential_tests.rs`)
//! pins decision + rule-name equivalence.

use super::ast::{Decision, Policy};
use super::ast_evaluator::ReapAstEvaluator;
use super::compiler;
use crate::data::DataStore;
use crate::evaluators::reaper_dsl::ReaperDSLEvaluator;
use crate::evaluators::{NamedOutcome, PolicyEvaluator};
use crate::PolicyAction;
use reaper_core::ReaperError;
use std::sync::Arc;

/// One rule, on whichever side of the compile line it landed.
#[derive(Debug)]
enum UnitEval {
    Compiled(ReaperDSLEvaluator),
    Ast(Box<ReapAstEvaluator>),
}

#[derive(Debug)]
struct RuleUnit {
    /// The rule's name, owned here so `evaluate_named` can borrow from self.
    name: String,
    eval: UnitEval,
}

impl RuleUnit {
    /// Evaluate this single rule: `Some(decision)` iff the rule MATCHED
    /// (named outcome carries a rule name); `None` when the sub-policy
    /// default answered (i.e. the rule did not match).
    fn matched_decision(
        &self,
        request: &crate::PolicyRequest,
    ) -> Result<Option<PolicyAction>, ReaperError> {
        let outcome = match &self.eval {
            UnitEval::Compiled(e) => e.evaluate_named(request)?,
            UnitEval::Ast(e) => e.evaluate_named(request)?,
        };
        Ok(outcome.rule_name.map(|_| outcome.decision))
    }
}

/// Per-rule mixed compiled/AST evaluator. Built by
/// [`MixedReapEvaluator::build`] via [`super::ReaperPolicy::build_preferred`]
/// when a policy does not compile whole but at least one rule compiles.
#[derive(Debug)]
pub struct MixedReapEvaluator {
    deny_units: Vec<RuleUnit>,
    allow_units: Vec<RuleUnit>,
    default_decision: PolicyAction,
    /// Names of the rules that fell back to the interpreter, for
    /// observability (deploy logs, metadata).
    ast_rule_names: Vec<String>,
    total_rules: usize,
    /// The store, for the unknown-principal routing check below.
    store: Arc<DataStore>,
    /// Whole-policy interpreter for the UNKNOWN-PRINCIPAL edge. The compiled
    /// evaluator errors at entry when the request's principal is not a
    /// loaded entity (a deliberate primary-path contract this feature must
    /// not disturb), while the interpreter reads absent entities as Null
    /// (fail-closed non-match). Every policy served mixed previously ran
    /// whole-AST, so on that edge the mixed evaluator must reproduce the
    /// AST outcome — compiled units cannot (they'd error before examining
    /// their rule). Requests whose principal does not resolve to a loaded
    /// entity are therefore routed here wholesale; everything else takes
    /// the fast per-rule path.
    whole_ast: Box<ReapAstEvaluator>,
}

/// Outcome of a per-rule build: mixing only pays when at least one rule
/// compiled; a policy where every rule needs the interpreter is served
/// (unchanged from before) by one whole-policy AST evaluator, which is
/// cheaper than N single-rule ones.
#[derive(Debug)]
pub enum PerRuleBuild {
    Mixed(MixedReapEvaluator),
    AllAst(ReapAstEvaluator),
}

impl MixedReapEvaluator {
    /// Split `policy` at rule granularity: compile each rule alone; rules
    /// the compiler rejects get a single-rule AST interpreter. Returns
    /// [`PerRuleBuild::AllAst`] when no rule compiled.
    pub fn build(policy: Policy, store: Arc<DataStore>) -> Result<PerRuleBuild, ReaperError> {
        let mut deny_units = Vec::new();
        let mut allow_units = Vec::new();
        let mut ast_rule_names = Vec::new();
        let total_rules = policy.rules.len();

        for rule in &policy.rules {
            let sub_policy = Policy {
                name: policy.name.clone(),
                metadata: policy.metadata.clone(),
                default_decision: policy.default_decision.clone(),
                rules: vec![rule.clone()],
            };
            let eval = match compiler::compile_policy(sub_policy.clone(), store.clone()) {
                Ok(compiled) => UnitEval::Compiled(compiled),
                Err(reason) => {
                    tracing::debug!(
                        policy = %policy.name,
                        rule = %rule.name,
                        %reason,
                        "rule falls back to the AST interpreter"
                    );
                    ast_rule_names.push(rule.name.clone());
                    UnitEval::Ast(Box::new(ReapAstEvaluator::new(store.clone(), sub_policy)))
                }
            };
            let unit = RuleUnit {
                name: rule.name.clone(),
                eval,
            };
            match rule.decision {
                Decision::Deny => deny_units.push(unit),
                Decision::Allow => allow_units.push(unit),
            }
        }

        if ast_rule_names.len() == total_rules {
            // Nothing compiled — one whole-policy interpreter beats N
            // single-rule ones (shared eval context, shared regex cache).
            return Ok(PerRuleBuild::AllAst(ReapAstEvaluator::new(store, policy)));
        }

        let default_decision = policy.default_decision.clone().into();
        let whole_ast = Box::new(ReapAstEvaluator::new(store.clone(), policy));
        Ok(PerRuleBuild::Mixed(Self {
            deny_units,
            allow_units,
            default_decision,
            ast_rule_names,
            total_rules,
            store,
            whole_ast,
        }))
    }

    /// Does the request's principal resolve to a LOADED entity? Mirrors the
    /// compiled evaluator's entry checks (`lookup`, never `intern` — a
    /// per-request principal must not be pinned in the shared interner).
    fn principal_is_loaded(&self, request: &crate::PolicyRequest) -> bool {
        request
            .context
            .get("principal")
            .and_then(|p| self.store.interner().lookup(p))
            .and_then(|id| self.store.get(id))
            .is_some()
    }

    /// (compiled, interpreted) rule counts — observability surface.
    pub fn rule_modes(&self) -> (usize, usize) {
        (
            self.total_rules - self.ast_rule_names.len(),
            self.ast_rule_names.len(),
        )
    }

    /// Names of the rules running on the interpreter.
    pub fn ast_rule_names(&self) -> &[String] {
        &self.ast_rule_names
    }

    /// Like [`PolicyEvaluator::evaluate_named`] but with a structured
    /// `input` document (R4-01 B.1). No unknown-principal routing here: the
    /// compiled `*_with_input` entry synthesizes an empty principal (Null
    /// reads) exactly like the interpreter, so the per-rule units agree on
    /// that edge by construction and both document- and entity-anchored
    /// rules dispatch to their fast side.
    pub fn evaluate_with_input_named(
        &self,
        request: &crate::PolicyRequest,
        input: Option<&serde_json::Value>,
    ) -> Result<(PolicyAction, Option<&str>), ReaperError> {
        let unit_matched = |unit: &RuleUnit| -> Result<bool, ReaperError> {
            Ok(match &unit.eval {
                UnitEval::Compiled(e) => e.evaluate_with_input_named(request, input)?.1.is_some(),
                UnitEval::Ast(e) => e.evaluate_with_input_named(request, input)?.1.is_some(),
            })
        };
        for unit in &self.deny_units {
            if unit_matched(unit)? {
                return Ok((PolicyAction::Deny, Some(unit.name.as_str())));
            }
        }
        for unit in &self.allow_units {
            if unit_matched(unit)? {
                return Ok((PolicyAction::Allow, Some(unit.name.as_str())));
            }
        }
        Ok((self.default_decision.clone(), None))
    }

    /// Check-mode over the mixed split (R4-01 B.3): each deny unit
    /// contributes its violations in source order (a single-rule sub-policy
    /// yields exactly that rule's violation when it matches); `allowed`
    /// composes exactly like the interpreter's check driver.
    pub fn check_with_input(
        &self,
        request: &crate::PolicyRequest,
        input: Option<&serde_json::Value>,
    ) -> Result<super::CheckResult, ReaperError> {
        let mut violations = Vec::new();
        for unit in &self.deny_units {
            let result = match &unit.eval {
                UnitEval::Compiled(e) => e.check_with_input(request, input)?,
                UnitEval::Ast(e) => e.check_with_input(request, input)?,
            };
            violations.extend(result.violations);
        }
        let allowed = if violations.is_empty() {
            match self.default_decision {
                PolicyAction::Allow => true,
                _ => {
                    let mut any = false;
                    for unit in &self.allow_units {
                        let matched = match &unit.eval {
                            UnitEval::Compiled(e) => {
                                e.evaluate_with_input_named(request, input)?.1.is_some()
                            }
                            UnitEval::Ast(e) => {
                                e.evaluate_with_input_named(request, input)?.1.is_some()
                            }
                        };
                        if matched {
                            any = true;
                            break;
                        }
                    }
                    any
                }
            }
        } else {
            false
        };
        Ok(super::CheckResult {
            allowed,
            violations,
        })
    }

    /// The shared deny-overrides / first-allow-wins / default loop.
    fn decide(
        &self,
        request: &crate::PolicyRequest,
    ) -> Result<(PolicyAction, Option<&str>), ReaperError> {
        if !self.principal_is_loaded(request) {
            // Unknown-principal edge: reproduce the whole-AST outcome this
            // policy had before mixed mode (see the `whole_ast` field docs).
            let (action, name) = self.whole_ast.evaluate_with_input_named(request, None)?;
            return Ok((action, name));
        }
        for unit in &self.deny_units {
            if let Some(decision) = unit.matched_decision(request)? {
                debug_assert!(matches!(decision, PolicyAction::Deny));
                return Ok((PolicyAction::Deny, Some(unit.name.as_str())));
            }
        }
        for unit in &self.allow_units {
            if let Some(decision) = unit.matched_decision(request)? {
                return Ok((decision, Some(unit.name.as_str())));
            }
        }
        Ok((self.default_decision.clone(), None))
    }
}

impl PolicyEvaluator for MixedReapEvaluator {
    fn evaluate(&self, request: &crate::PolicyRequest) -> Result<PolicyAction, ReaperError> {
        self.decide(request).map(|(action, _)| action)
    }

    /// Always-decisive, mirroring the AST fallback this replaces (see module
    /// docs) — set-combination semantics must not change under a speed
    /// feature.
    fn evaluate_matched(
        &self,
        request: &crate::PolicyRequest,
    ) -> Result<(PolicyAction, bool), ReaperError> {
        self.decide(request).map(|(action, _)| (action, true))
    }

    fn evaluate_named(
        &self,
        request: &crate::PolicyRequest,
    ) -> Result<NamedOutcome<'_>, ReaperError> {
        let (decision, rule_name) = self.decide(request)?;
        Ok(NamedOutcome {
            decision,
            matched: true,
            rule_name,
        })
    }

    fn check_with_input(
        &self,
        request: &crate::PolicyRequest,
        input: Option<&serde_json::Value>,
    ) -> Result<crate::reap::CheckResult, ReaperError> {
        MixedReapEvaluator::check_with_input(self, request, input)
    }

    fn validate(&self) -> Result<(), ReaperError> {
        if self.total_rules == 0 {
            return Err(ReaperError::InvalidPolicy {
                reason: "Policy must have at least one rule".to_string(),
            });
        }
        for unit in self.deny_units.iter().chain(self.allow_units.iter()) {
            match &unit.eval {
                UnitEval::Compiled(e) => e.validate()?,
                UnitEval::Ast(e) => e.validate()?,
            }
        }
        Ok(())
    }

    fn evaluator_type(&self) -> &str {
        "reaper_dsl_mixed"
    }

    fn metadata(&self) -> Option<crate::evaluators::EvaluatorMetadata> {
        let (compiled, ast) = self.rule_modes();
        let mut extra = std::collections::HashMap::new();
        extra.insert("compiled_rules".to_string(), compiled.to_string());
        extra.insert("ast_rules".to_string(), ast.to_string());
        extra.insert("ast_rule_names".to_string(), self.ast_rule_names.join(","));
        Some(crate::evaluators::EvaluatorMetadata {
            rule_count: self.total_rules,
            complexity: 50,
            extra,
        })
    }

    // resource_index_terms / resource_pruning: trait defaults (None /
    // Unprunable) — the identical bound the whole-AST policy had. See module
    // docs for why this is deliberately unchanged.
}
