# Multi-tenant SaaS — tenant walls (ABAC) + sharing graphs (ReBAC) + roles (RBAC)

The pattern every B2B product needs and no single model covers:

- **ABAC as the isolation wall**: `tenant_isolation` is a *deny* rule on
  attribute inequality — it fires before any allow can leak data across
  tenants. One line, mathematically ahead of every other rule (deny wins).
- **ReBAC inside the tenant**: project access flows through membership edges
  (`user → squad → project`), watchers get read-only via a direct edge.
  Sharing UX = writing edges, not editing policy.
- **RBAC at the edges**: `tenant_admin` within their tenant; `support` staff
  can cross tenants but ONLY for reads and ONLY with a ticket in the request
  context — break-glass that leaves an audit trail (the ticket lands in the
  decision log).

**Try it**
```bash
reaper-cli library run combined/saas-tenancy
```
