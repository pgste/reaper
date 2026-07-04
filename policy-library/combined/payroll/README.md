# Payroll (the OPA "read own salary" classic)

Self-access via attribute equality, manager access via bounded `manages`
chain traversal — the example OPA docs implement with recursive Rego helper
rules, here a single `rebac::reachable(user, "subject", resource, "manages", 2)`.

Try: `reaper-cli library run combined/payroll`
