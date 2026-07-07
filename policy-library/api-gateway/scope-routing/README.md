# Path/method routing rules (Envoy ext_authz pattern)

**Pattern**: the gateway sends `{path, method, token}` for every request;
policy decides per route class. Health endpoints are open, public API is
read-only-open, admin routes require the `admin` scope from the JWT, and
`/internal/*` is deny-with-message even if something else would allow it —
demonstrating deny-wins layering.

**What to look at**
- `input.path.startswith(...)` — string methods directly on document fields.
- Mixing anonymous routes and JWT-scoped routes in one policy.
- The `internal_paths_never_via_gateway` deny rule fires *with a rendered
  message* containing the offending path (`p := input.path` binds it for the
  message).

**Try it**
```bash
reaper-cli library run api-gateway/scope-routing
```
