# Governed Promotion (Two-Person Control)

Promoting a bundle is the moment new policy goes live on every agent. Reaper can
gate that moment behind **two-person control** (four-eyes / separation of
duties): a promotion is opened by one principal and must be **approved by a
different one** before it takes effect. Every promotion — governed or not — is
written to an immutable **change record** and the audit log, so *"who requested
what, approved by whom, when"* is always answerable.

This is designed for enterprise change management: the approver can be a person,
a **group** (a change-approval board), or a **service account** driven by an
external system (ServiceNow, a CI/CD promotion job), and identities link back to
your **corporate directory** through SSO.

## Posture: opt-in, on-by-default for the managed profile

The behaviour is chosen per deployment, because "secure by default" and "works
out of the box" pull against each other — forcing two principals would break a
solo operator, a CI service account, or a sidecar with a single identity.

| Mode | What a promote does | Who it's for |
|------|--------------------|--------------|
| **Single-control** (code default) | Caller with `bundle:promote` promotes immediately (a change record is still written) | Solo operators, CI, sidecar/engine, OPA-style deployments |
| **Dual-control** (managed-profile default) | Promote/rollback open a *pending* change request a **different** principal must approve | Enterprises with a change process |

The **managed control plane ships dual-control on** (Helm chart + docker
`management` profile), so enterprises get four-eyes by default and opt *out*;
lightweight deployments keep single-control and opt *in*. The change ledger is
always on — only the approval gate varies.

## The separation-of-duties scopes

Three distinct authorities, each independently grantable:

| Scope | Grants the authority to… |
|-------|--------------------------|
| `bundle:promote` | **Request** a promotion/rollback (and, under single-control, execute it directly) |
| `bundle:approve` | **Approve** and execute a pending change request |
| `bundle:read` | View bundles and change requests |

Keeping `bundle:approve` **separate** from `bundle:promote` is what makes true
separation of duties possible: give the deploy pipeline `bundle:promote` and the
change-approval board `bundle:approve`, and no single principal can both
originate and approve a promotion. Rejection (declining or withdrawing a request)
accepts **either** scope.

Built-in org roles map as follows:

| Role | promote | approve | Notes |
|------|:---:|:---:|-------|
| **Owner** | ✅ | ✅ | Full control — request and approve |
| **Admin** (org) | — | ✅ | Cannot originate a promotion; the built-in **approver** role |
| **Developer** | — | — | Authors/stages bundles (`bundle:write`) |
| **Viewer** | — | — | Read-only |

> For a real change-approval board, grant `bundle:approve` to a dedicated **IdP
> group** or **API key** *without* `bundle:promote` (see
> [Corporate login](#corporate-login--groups)), rather than relying on the Owner
> role. The platform `admin` scope covers all authorities.

## How dual-control works

```
requester (bundle:promote)                approver (bundle:approve, ≠ requester)
    │                                                │
    ├─ POST /bundles/{id}/promote  ──▶ 201 pending   │
    │      (records who / which bundle content)      │
    │                                                │
    │                       POST /change-requests/{id}/approve ─┐
    │                                                           ▼
    │                                   atomic claim (pending→executed)
    │                                   then promote  ──▶ 200 live
    └─ (or) POST .../reject  ──▶ request withdrawn
```

- **Distinct principal:** unless `allow_self_approval` is set, the approver must
  not be the requester (self-approval → `403`). Compared on the stable principal
  id — the corporate SSO subject for JWT/SSO callers, the key id for service
  accounts.
- **Atomic execution:** the `pending → executed` transition is a single guarded
  `UPDATE`, so two concurrent approvals can't both promote — the loser gets a
  `409`.
- **Verifiable rollback:** `POST /bundles/{id}/rollback` opens a rollback change
  request that re-promotes a previously-good (Deprecated/Staged) bundle under the
  same two-person control, archiving the current live one.

### Endpoints

```
POST /orgs/{org}/bundles/{id}/promote      # open a promote request  (or promote now, single-control)
POST /orgs/{org}/bundles/{id}/rollback     # open a rollback request
GET  /orgs/{org}/change-requests           # list change records (newest first)
GET  /orgs/{org}/change-requests/{id}      # one change record
POST /orgs/{org}/change-requests/{id}/approve
POST /orgs/{org}/change-requests/{id}/reject
```

## Configuration

Environment variables (control plane / reaper-management):

```bash
REAPER_PROMOTION_APPROVAL=dual_control        # or "disabled" (single-control, the code default)
REAPER_PROMOTION_ALLOW_SELF_APPROVAL=false    # true only for a lone automated service account
```

`REAPER_PROMOTION_APPROVAL` accepts `disabled` (aliases: `single`, `off`) and
`dual_control` (aliases: `dual`, `two_person`, `four_eyes`, `on`).

**Helm** (`values.yaml`) — the managed chart defaults to dual-control:

```yaml
management:
  config:
    promotionApproval: dual_control   # set "disabled" for single-control
    allowSelfApproval: false
```

**docker-compose** — the `management` profile defaults to dual-control; override
per-deployment:

```bash
PROMOTION_APPROVAL=disabled docker compose --profile management up -d
```

### `allow_self_approval`

Off by default — this is what makes four-eyes meaningful. Turn it on **only** for
a fully-automated pipeline where the real approval gate lives *outside* Reaper (a
CI/CD promotion job, a ServiceNow change record) and a single service account
both opens and executes the change. The change is still recorded; the distinct-
principal constraint is intentionally relaxed.

## Corporate login & groups

Approval authority ties straight back to your corporate directory — no extra
plumbing:

- **A change-approval board (CAB)** is an **IdP group**. With OIDC/JWKS, the
  token's `groups`/`roles` claims map into scopes, so mapping the CAB group to
  `bundle:approve` means *any member* can approve — membership is managed
  entirely in Okta / Entra ID / your IdP.
- **A systems/service account** is an **API key** with `bundle:approve` (e.g.
  ServiceNow decides, then its account calls `approve`). It is a distinct
  principal from the human requester, so four-eyes holds.
- **The audit trail links to the directory:** the `requester_id` / `approver_id`
  recorded on every change request are the corporate **SSO subject** (or key id
  for service accounts), so "who requested / who approved" resolves to a real
  directory identity.

## Change records & audit

Every promotion and rollback — in either mode — writes a
`promotion_change_requests` row (id, org, bundle id, pinned bundle content
checksum, kind, status, requester, approver, timestamps) and an audit-log entry
on each transition (`open`, `approve`, `reject`, `promote`). The bundle content
is pinned by **checksum** at request time, so the record names *which artifact*
was approved, not merely its id.

Combined with [Bundle Signing](./BUNDLE_SIGNING.md), promotion is governed
end-to-end: a distinct approver authorizes *that* the policy goes live, and the
signature guarantees agents only load the exact bundle the control plane
produced.
