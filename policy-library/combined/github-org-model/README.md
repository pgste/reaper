# GitHub-style org/team/repo model — why RBAC + ABAC + ReBAC compose

Most OPA docs show each model in isolation. Real products need all three in
the SAME decision, and this scenario is the clearest demonstration:

> May `dev-sam` push to `repo-payments`?

1. **ReBAC** answers *"does a team grant reach this user?"* —
   `repo-payments #write @team-payments`, and sam is `member_of`
   `team-payments` (nesting up to 3 hops, so team-in-team works).
2. **ABAC** answers *"is the account itself acceptable?"* — private-repo
   pushes require `mfa_enabled == true`. Sam with MFA pushes; contractor-cody
   in the same team without MFA cannot. A pure ReBAC system (raw Zanzibar)
   cannot express this without duplicating tuples per attribute.
3. **RBAC** answers the org-shaped exceptions — `org_admin` bypasses grants;
   `bot` accounts are deny-listed from `force-push` even when a team grant
   and MFA would allow it (deny wins).

In Rego, this takes ~60 lines with hand-written graph-walk helpers and no
traversal bounds. Here it is 5 rules, and the ReBAC hop is a ~110ns compiled
graph lookup.

**Try it**
```bash
reaper-cli library run combined/github-org-model
```
