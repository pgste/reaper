package reaper.abac

import rego.v1

# ABAC Policy — mirrors abac_clearance.reap exactly
# Rules:
#   1. deny_suspended_users: deny if user.suspended == true
#   2. clearance_and_department: user.clearance_match && same dept && !archived
#   3. high_clearance_access: user.high_clearance && same dept && !secret && !archived
#   4. owner_access: user.id == resource.owner_id && user.status == "active"
#   5. executive_access: user.role == "executive" && !archived

default allow := false

# Entity lookups — .attributes shorthand so user.role works directly
user := data.entities[input.principal.id].attributes

resource := data.entities[input.resource].attributes

# Deny suspended users immediately (highest priority)
deny if {
    user.suspended == true
}

# Allow same department with matching clearance level
allow if {
    not deny
    user.clearance_match == true
    user.department == resource.department
    resource.archived != true
}

# High clearance users can access confidential docs in their dept (not archived)
allow if {
    not deny
    user.high_clearance == true
    user.department == resource.department
    resource.classification != "secret"
    resource.archived != true
}

# Document owners can always access (unless suspended)
allow if {
    not deny
    user.id == resource.owner_id
    user.status == "active"
}

# Executive full access (except archived)
allow if {
    not deny
    user.role == "executive"
    resource.archived != true
}
