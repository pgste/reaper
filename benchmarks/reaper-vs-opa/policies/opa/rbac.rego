package reaper.rbac

import rego.v1

# RBAC Policy — mirrors rbac_simple.reap exactly
# Rules:
#   1. admin_full_access: user.role == "admin"
#   2. manager_reports: user.role == "manager" && resource.type == "report"
#   3. user_own_resources: user.id == resource.owner_id

default allow := false

# Entity lookups — .attributes shorthand
user := data.entities[input.principal.id].attributes

resource := data.entities[input.resource].attributes

# Admins can do anything
allow if {
    user.role == "admin"
}

# Managers can read reports
allow if {
    user.role == "manager"
    resource.type == "report"
}

# Users can access their own resources
allow if {
    user.id == resource.owner_id
}
