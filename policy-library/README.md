# Reaper Policy Library

Real-world authorization and guardrail scenarios, adapted from the most-cited
OPA/Rego examples and rewritten in Reaper's `.reap` DSL — including things
Rego cannot express natively (first-class ReBAC graph traversal) and the
things it's famous for (Terraform plan and Kubernetes admission checks).

Every scenario is executable and CI-tested: `manifest.json` declares the
expected decision for each case, and `crates/policy-engine/tests/
policy_library_tests.rs` runs them all — authorization cases through BOTH the
compiled and AST evaluators (asserting they agree), document cases through
check mode (asserting the exact violation set).

| Scenario | Models | Adapted from |
|----------|--------|--------------|
| `rbac/role-based-access` | RBAC | OPA docs "RBAC" tutorial (role → permission mapping) |
| `abac/banking-accounts` | ABAC | OPA docs "ABAC" tutorial (attributes of subject + resource) |
| `rebac/document-sharing` | ReBAC | Google Zanzibar / Google-Drive-style sharing (OPA needs hand-rolled graph walks for this; Reaper has `rebac::*` builtins) |
| `rebac/manager-approval` | ReBAC | Classic org-chart approval chain (Styra blog pattern) |
| `terraform/s3-guardrails` | document | OPA "Terraform" tutorial + conftest AWS examples |
| `kubernetes/admission-control` | document | OPA Gatekeeper library staples (disallowed tags, required labels, privilege, registries) |
| `combined/healthcare-records` | RBAC+ABAC+ReBAC | HL7/consent patterns; the three models in ONE rule |
| `combined/payroll` | ABAC+ReBAC | OPA's canonical "employees can read their own salary; managers their reports'" example |

## Try one

```bash
# document scenarios (CI mode):
reaper-cli check -p policy-library/terraform/s3-guardrails/policy.reap \
    -i policy-library/terraform/s3-guardrails/input-violating.json

# authorization scenarios:
reaper-cli eval --policy policy-library/combined/healthcare-records/policy.reap \
    --data policy-library/combined/healthcare-records/data.json \
    --principal dr-adams --action read --resource record-101
```

## Manifest format

```json
{
  "name": "human name",
  "source": "where the scenario is adapted from",
  "policy": "policy.reap",
  "data": "data.json",
  "cases": [
    {"name": "...", "principal": "alice", "action": "read", "resource": "doc1", "expect": "allow"},
    {"name": "...", "input": "input-violating.json", "expect": "deny", "violations": ["rule_name"]}
  ]
}
```

## Performance (measured)

`cargo bench -p policy-engine --bench rebac_bench` on a 16k-entity graph
(10k users, 1k groups nested 3 deep, 5k docs in a folder tree):

| Check | Time |
|-------|------|
| direct relation (`rebac::related`) | ~18 ns |
| group expansion hit, 1 hop (`rebac::reachable`) | ~110 ns |
| bounded 3-hop expansion, worst-case miss | ~246 ns |
| folder inheritance hit (`rebac::inherited`) | ~118 ns |
| **full compiled policy eval** (3 ReBAC rules, allow) | **~267 ns** |
| full compiled policy eval (deny, all rules swept) | ~539 ns |
| same policy on the AST evaluator | ~914 ns |

Both paths are sub-microsecond; equivalent Rego graph-walk policies typically
run in the tens of microseconds to milliseconds.
