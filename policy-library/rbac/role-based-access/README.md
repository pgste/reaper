# Role-based access (OPA RBAC tutorial)

Users hold roles; per-role rules grant actions; `default: deny`. The one
subtlety worth studying: `suspended_never` is a **deny rule and deny wins** —
a suspended admin stays locked out even though `admins_do_anything` matches.

Try: `reaper-cli library run rbac/role-based-access`
