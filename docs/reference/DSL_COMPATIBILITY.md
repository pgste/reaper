# Reaper DSL Compatibility & Deprecation Policy

The `.reap` policy language is a **public contract**. Customers author policies
against it and auditors read them, so a policy committed today must keep
producing the **same authorization decision** on every future engine version —
or, if a decision must ever change, it changes **loudly**, behind an explicit
language-version bump, never silently.

This document defines what "compatible" means, how the guarantee is enforced,
and how the language is allowed to evolve.

## The core guarantee

> For a fixed `(policy, data, request)`, the decision (`allow`/`deny`) is stable
> across engine versions unless the policy's declared language version changes.

This is enforced mechanically, not by convention:

- **Frozen decision corpus** (`policy-library/frozen/`, run by
  `crates/policy-engine/tests/frozen_decision_corpus_tests.rs`, blocking in CI).
  Every frozen `(policy, data, request) → decision` case runs through **both**
  the AST and compiled evaluators. A change that alters a frozen decision — even
  one applied to *both* paths, which the compiled-vs-AST differential cannot
  catch — turns the build red.
- **Immutability gate.** Every frozen file is checksummed in
  `policy-library/frozen/CHECKSUMS`. Editing a frozen expectation forces a
  visible `CHECKSUMS` diff, so a decision change can only land through a
  deliberate, reviewed act — never a silent manifest edit.

## Breaking vs additive changes

**Breaking** = *any* change to the decision of an already-deployable policy.
Examples: changing an operator's semantics (e.g. `==` type-strictness),
re-scoping a builtin's result, altering rule-ordering or default-decision
behaviour, or removing a keyword/operator/builtin.

- A breaking change **requires a language-version bump**
  (`CURRENT_LANGUAGE_VERSION` in `crates/policy-engine/src/reap/mod.rs`) plus a
  dated waiver in this file recording which frozen cases changed and why.
- Old engines **fail closed** on a newer-versioned policy
  (`ReaperError::LanguageVersionUnsupported`) — they never down-level or
  best-effort parse it. This mirrors the bundle wire format's newer-version
  reject one layer down (`reap/bundle.rs`).

**Additive** = a change that does **not** alter any already-deployable policy's
decision: a new keyword, operator, builtin, or a new *optional* syntax.

- Additive changes must keep the frozen corpus green (the proof they are
  decision-neutral). They do **not** require a version bump, but authors are
  encouraged to declare `language_version` so intent is explicit.

## Declaring a language version

A policy declares its target version with a metadata field:

```reap
policy my_policy {
    language_version: "2",
    default: deny,
    // rules...
}
```

A policy that declares **no** version is treated as the current implicit
version. A policy that declares a version **newer** than the engine implements
is rejected at parse time and at bundle load — fail-closed, never
misinterpreted.

## Deprecation window

When a keyword/operator/builtin is to be removed:

1. It is marked deprecated for a window of **at least 2 releases**.
2. During the window it keeps its frozen decision (deprecation ≠ removal) and
   emits a machine-readable warning at parse/compile with a `since` /
   `removed_in` tag, so tooling can surface it.
3. Removal is a breaking change: a language-version bump, in a release no
   earlier than the announced `removed_in`.

## Waiver log

Record every intentional frozen-decision change here: date, PR, the language
version it bumped to, and the frozen cases affected.

| Date | PR | Language version | Frozen cases changed | Reason |
|------|----|------------------|----------------------|--------|
| _(none yet — the frozen corpus is at its initial baseline)_ | | | | |

## Deprecated surfaces

- **`Simple` policy language** — deprecated and unsafe by design (it matches on
  resource only, ignoring the request action, principal/context, and a rule's
  conditions). It is **not** a co-equal authorization language and should not be
  used for new policies; author policies in the `.reap` DSL. Removal is tracked
  with the JSON-rule migration.
