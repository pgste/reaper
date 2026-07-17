# Healthcare records — RBAC + ABAC + ReBAC in single rules

`treating_physician_reads` needs all three models in ONE condition: role
(RBAC), department + clearance-vs-sensitivity (ABAC), and the
treating-relationship edge (ReBAC). Consent revocation is a deny-wins rule
that overrides every allow, including auditors.

Try: `reaper-cli library run combined/healthcare-records`
