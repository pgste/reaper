# Release gates — comprehensions over entity data

The terraform and kubernetes scenarios run comprehensions over an `input`
document. This scenario runs the SAME language feature over lists of objects
stored on a **resource entity** — CI checks and human approvals — and is
evaluated by both the compiled evaluator and the AST interpreter (the
library test runner asserts they agree on every case).

## The policy

```reap
rule failed_checks_block_release {
    deny if {
        bad := [c.name | c := resource.checks[_]; c.status == "failed"] &&
        bad.count() > 0
    }
}
```

An array comprehension walks `resource.checks`, keeps entries whose
`status` is `"failed"`, and the `count()` guard turns "any failures?" into
a decision. Deny rules always win, so a failed check freezes the release
even for the release-manager fast path.

## Semantics worth noticing

- **Deny wins**: `svc-payments` has two lead approvals AND a failed load
  test — the deny rule fires and the approvals are irrelevant.
- **Total iteration**: `svc-legacy` has no `checks` attribute at all. The
  comprehension iterates an EMPTY collection — no error, no accidental
  match — so only the approvals rules decide. Absence never satisfies a
  guard and never fails the evaluation.
- **Role fast path**: the same `approvals` list feeds two different allow
  rules with different thresholds, gated by `user.role`.

## Run it

```bash
reaper-cli library run combined/release-gates
```
