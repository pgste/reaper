# Multi-Tenant Plan Quotas

*Round-2 workstream E4. Closes PROD R2-9.*

Plan limits used to be advisory — `UsageMetrics` was hardcoded to `0` and nothing
was enforced, so any org could register unlimited agents and policies regardless
of tier. E4 makes them **real**: usage is counted from the database and the
limits are enforced at the create paths.

## Plans & limits

Tiers and their limits are defined in `domain/billing.rs` (`PlanTier`,
`PlanLimits::for_tier`):

| Tier | max agents | max policies | max users |
|------|-----------|--------------|-----------|
| `free` | 2 | 10 | 3 |
| `starter` | 10 | 50 | 10 |
| `professional` | 50 | 200 | 50 |
| `enterprise` | ∞ (`-1`) | ∞ | ∞ |

A limit of `-1` (or `0`) means unlimited.

## Where the plan is stored

An org's plan is persisted in its **`settings` JSON** (no new table):

```json
{ "plan_tier": "professional" }
```

Set it via the org update path (`PUT /orgs/{org}` with `settings`). A missing or
unknown `plan_tier` falls back to **`free`** (the tightest limits — fail-safe).

### Per-org overrides

For a custom enterprise deal, raise a single limit without changing tier by
adding a numeric override to `settings` — `max_agents`, `max_policies`,
`max_users`, `max_storage_bytes`, `max_evaluations_per_month`:

```json
{ "plan_tier": "starter", "max_agents": 25 }
```

## Enforcement

Quota is checked **before** the insert, from a live `SELECT COUNT(*)`:

- **Agent registration** — `POST /orgs/{org}/agents/register`
- **Policy creation** — `POST /orgs/{org}/policies`

When creating one more would exceed the limit, the request is refused with
**`402 Payment Required`** (`code: "quota_exceeded"`), e.g.:

```
agents quota reached: 2/2 on the free plan — upgrade the plan or raise the limit to add more
```

`402` is deliberate: the request is well-formed and authorized — the remedy is to
upgrade the plan (or raise the override), not to re-authenticate.

## Visibility

`GET /orgs/{org}/billing` reports the org's effective `plan_tier`, `limits`, real
`usage` counts, and any `exceeded_limits` — so an operator can see headroom
before hitting a wall.

## Not counted at the control plane

`policy_evaluations` and `storage_bytes` are data-plane metrics (they happen on
agents), so they are reported as `0` here and are not enforced by the control
plane. `active_agents`, `policy_count`, `bundle_count`, and `user_count` are
real counts.

## Implementation

`services/reaper-management/src/quota/mod.rs`: `plan_tier_of`, `effective_limits`,
`count_usage`, and `enforce_can_add(db, org, Dimension)`. Counts come from
`{Agent,Policy,Bundle,Team}Repository::count_by_org`.
