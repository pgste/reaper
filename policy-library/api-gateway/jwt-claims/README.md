# JWT claims validation at the API gateway

**Pattern**: the gateway (Envoy/Kong/nginx) verifies the JWT *signature* at
the trust boundary, then asks Reaper whether this **request + claims**
combination is allowed. The policy pins the issuer and audience, enforces
expiry, and maps HTTP methods to OAuth2 scopes.

**What to look at**
- `jwt::decode(input.token)` — parses the compact JWS payload into an object
  (OPA `io.jwt.decode` parity, including `Bearer ` prefix handling). Malformed
  tokens decode to `null`, so every rule fails closed — no 500s from garbage
  tokens.
- `claims.scope.split(" ")` + `"orders:read" in scopes` — the standard OAuth2
  space-delimited scope check.
- `time::now_secs()` vs `claims.exp` — JWT expiry is epoch seconds.
- `reject_alg_none` — a **deny** rule (deny wins): even if some allow rule
  matched, an `alg=none` token is rejected outright.

**Try it**
```bash
reaper-cli library run api-gateway/jwt-claims
```
