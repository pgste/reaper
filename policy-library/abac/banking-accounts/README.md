# Attribute-based banking access (OPA ABAC tutorial)

No role tables — decisions come from attributes of the subject (`title`,
`branch`, `seniority`) and the resource (`branch`, `owner`, `balance`).
Note `context.principal == resource.owner` (owner access without any role
machinery) and cross-entity compares like `user.branch == resource.branch`.

Try: `reaper-cli library run abac/banking-accounts`
