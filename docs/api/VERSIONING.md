# API Versioning & Deprecation Policy

Applies to the Reaper **control-plane API** (`reaper-management`) and the
**agent data-plane API** (`reaper-agent`). It defines the stability contract a
consumer can rely on, how changes are rolled out, and how endpoints are retired.

## 1. The versioned surface

- The resource API is served under a single, explicit major-version prefix:
  **`/api/v1`**. There is no un-versioned surface — a request to a bare
  resource path (e.g. `GET /orgs`) returns **404**.
- **Probes are unversioned** and stay at the root: `/health`, `/health/*`,
  `/live`, `/ready`, `/metrics`, `/metrics/prometheus`, and `/openapi.json`.
  Orchestrators and spec discovery do not need to know the API version.
- The machine-readable contract is published at **`GET /openapi.json`** (OpenAPI
  3.1). Its `servers` entry is `/api/v1`, so the documented paths are relative to
  that base. A CI gate (`api-contract`) fails the build if a served route is
  missing from the spec or vice-versa.

## 2. What "v1" guarantees (the stability contract)

Within `v1`, these are **backward-compatible** and may ship at any time without a
version bump:

- Adding a new endpoint, or a new optional request field.
- Adding a new field to a response body.
- Adding a new optional query parameter with a safe default.
- Adding a new enum value to a field **documented as open** (clients must
  tolerate unknown values).
- Relaxing a constraint (e.g. a previously-required field becoming optional).

Consumers **must** be written to tolerate the above (ignore unknown response
fields; do not treat an added field/endpoint as a breaking change).

## 3. What counts as a breaking change

Any of the following requires a **new major version** (`/api/v2`), never an
in-place change to `v1`:

- Removing or renaming an endpoint, field, or query parameter.
- Making a previously-optional request field required, or tightening validation
  so previously-valid input is rejected.
- Changing a field's type, units, or semantics.
- Changing the type/shape of a response, or removing a response field.
- Changing an error's HTTP status code for an existing condition.
- Changing default behavior in a way an existing client would observe.

## 4. Deprecation & sunset

When an endpoint or field is slated for removal:

1. It is marked **deprecated** in the OpenAPI document (`deprecated: true`) and
   in the changelog, with the successor documented.
2. Responses carry deprecation signaling (RFC 8594):
   - `Deprecation: true`
   - `Link: <successor>; rel="successor-version"`
   - a `Warning: 299 …` human-readable note.
   - a `Sunset: <HTTP-date>` header once a removal date is set.
3. The deprecation window is **≥ 180 days** from the first release that ships the
   `Deprecation` header to the earliest removal. Security-driven removals may be
   faster, announced explicitly.
4. Removal happens only in a subsequent release after the window elapses.

### The bare-root alias (transitional)

The pre-`/api/v1` un-versioned layout is **off by default**. It can be re-enabled
for one release as a migration aid with `REAPER_SERVE_ROOT_ALIAS=true` (or
`server.serve_root_alias = true`): the resource API is then also served at the
bare root, tagged with the deprecation headers above. This is a temporary
rollback lever, not a supported surface — migrate clients to `/api/v1`.

## 5. Optimistic concurrency (`ETag` / `If-Match`)

Governed mutable resources — today **policies** and **bundles** — carry an
`ETag` on every `GET`/`PUT` response:

- **Policies**: the current version's content hash.
- **Bundles**: the current modification stamp.

A `PUT` must echo the ETag it read via `If-Match`. If a concurrent writer got
there first, the request fails with **412 Precondition Failed** — re-`GET` for
the fresh ETag, re-apply your change, and retry. `If-Match: *` means "the
resource exists in some state" and always writes guarded against the state read
at request time.

Enforcement is transitional: in the current release a `PUT` without `If-Match`
still succeeds (unguarded, logged as deprecated). Once
`server.require_if_match` flips on (next release, or today via
`REAPER_REQUIRE_IF_MATCH=true`), a missing `If-Match` is rejected with
**428 Precondition Required**. SDK/CLI/automation should start sending
`If-Match` now.

## 6. Idempotency keys (`Idempotency-Key`)

Propagation-triggering POSTs — **bundle promote/rollback**, **rollout create**,
and **org create** — accept an optional `Idempotency-Key` header so automation
can retry a timed-out request safely:

- The first execution stores its response; a **replay** of the same key within
  the retention window (default 48 h, `REAPER_IDEMPOTENCY_RETENTION_SECS`)
  returns the stored response verbatim, marked `Idempotency-Replayed: true`,
  and triggers **nothing**.
- The same key with a **different request** is rejected with **422** — one key
  per distinct operation.
- If the original request is still in flight, a duplicate gets **409**; retry
  shortly.
- Failed operations are not memoized: the same key may be retried.

## 7. Client guidance

- Always call under `/api/v1`; treat a `Deprecation` header as a signal to
  migrate before the `Sunset` date.
- Ignore unknown response fields; tolerate unknown enum values on open fields.
- Pin to the published OpenAPI document (`/openapi.json`) for code generation;
  regenerate on each release to pick up additive changes.
- On `PUT`, echo the last-read `ETag` as `If-Match`; on 412, re-read and retry.
- Send an `Idempotency-Key` (a fresh UUID per logical operation) on promote,
  rollback, rollout-create, and org-create; retry with the SAME key after a
  timeout.
