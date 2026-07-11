# API Route-Modeling Conventions

How new control-plane and agent endpoints are shaped (Plan 07, Phase F). The
goal is a surface that stays coherent as it grows: a reviewer should be able to
predict an endpoint's path, method, status codes, and envelope before reading
the handler.

## 1. Resources, not verbs

- Model nouns as **resources** under their owning scope:
  `GET/POST /orgs/{org}/policies`, `GET/PUT/DELETE /orgs/{org}/policies/{policy}`.
- Model operations as **action sub-resources** on the resource they act on —
  `POST /orgs/{org}/bundles/{id}/promote` — never as free-floating verb routes
  (`/promotePolicy`, `/init-all`). An action sub-resource is a POST; its
  response is the resource state or the record the action created (e.g. a
  change request).
- Prefer a plural noun for the collection and the bare id segment for the item.
  Path parameters accept the id, and — where the resource has one — the slug.

## 2. Methods & status codes

| Intent | Method | Success |
|---|---|---|
| List a collection | GET | 200, `Paginated` envelope |
| Read one | GET | 200 (+ `ETag` where the resource is editable) |
| Create | POST | 201 |
| Full update | PUT (+ `If-Match`) | 200 (+ new `ETag`) |
| Delete | DELETE | 204 |
| Action sub-resource | POST | 200/201/202 per the action's semantics |

- Errors are RFC 9457 `application/problem+json` (see `VERSIONING.md` §
  clients): `type`, `title`, `status`, `detail`, plus the `code` extension.
- Constraint breaches are client errors: unique → **409**, check/reference →
  **422** — never a 500.

## 3. Pagination

Every collection GET is **bounded**: `limit` (default 50, max 200 — an
over-max limit is a **400**, not a silent clamp) and an opaque keyset `cursor`.
Responses use the uniform envelope:

```json
{ "items": [ ... ], "next_cursor": "opaque-or-absent" }
```

Cursors are keyset positions (no OFFSET: it drifts under concurrent inserts
and degrades on deep pages); their format is not contract. The decisions
store uses a larger max (1000) to suit audit export patterns.

## 4. Retry-safety

Propagation-triggering POSTs (promote/rollback, rollout-create, org-create,
and any future endpoint whose double-execution is user-visible) accept an
`Idempotency-Key` header — see `VERSIONING.md` § idempotency.

## 5. Existing deviations (grandfathered, migrate on next major)

These predate the convention and stay for `v1` under the deprecation policy;
`v2` renames them to action sub-resource form:

| Today | Convention form |
|---|---|
| `POST /orgs/{org}/bundles/{id}/compile`, `/stage`, `/promote`, `/rollback`, `/deprecate` | Keep — already action sub-resources |
| `POST /orgs/{org}/rollback` (org-wide) | `POST /orgs/{org}/rollbacks` |
| `POST /orgs/{org}/agents/{id}/deployment/acknowledge` | `POST /orgs/{org}/agents/{id}/deployment-acks` |
| Agent data plane `POST /api/v1/messages`, `/fast-messages`, `/batch-messages` | Keep — enforcement hot path is exempt from restructuring (perf > uniformity) |

## 6. Checklist for a new endpoint

1. Noun under its owning scope; action sub-resource for operations.
2. `#[utoipa::path]` annotation (the contract-parity gate fails the build
   without it) with params, statuses, and `security(...)`.
3. Collection GETs take `PageQuery` and return `Paginated<T>`.
4. Editable resources emit `ETag` and require `If-Match` on PUT.
5. Errors flow through `ApiError` (problem+json comes free).
6. Propagation POSTs wire `api::idempotency::run`.
