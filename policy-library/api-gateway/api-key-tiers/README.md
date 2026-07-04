# API key tiers (keys as ABAC entities)

**Pattern**: instead of encoding entitlements inside an opaque key string,
the key is an *entity* in the data plane with attributes (`tier`,
`environment`, `status`). The gateway authenticates the key and passes its id
as the principal; the policy is pure ABAC over key + resource attributes.

**What to look at**
- Revocation as a deny-wins rule: flipping one attribute kills the key
  everywhere on the next decision (~µs), no key rotation required.
- Tier laddering (`free` read-only, `pro` read/write, `enterprise`
  cross-environment) as attribute comparisons — adding a tier is data, not
  code.
- Environment scoping via cross-entity compare
  (`user.environment == resource.environment`).

**Try it**
```bash
reaper-cli library run api-gateway/api-key-tiers
```
