package reaper.abac

import future.keywords.if
import future.keywords.in

# Default deny
default allow := false

# Direct O(1) entity lookups (entities pre-indexed as map)
user := data.entities[input.principal]
resource := data.entities[input.resource]

# Deny suspended users immediately
allow := false if {
    user.attributes.suspended == true
}

# Admin with high clearance can access everything
allow := true if {
    user.attributes.role == "admin"
    user.attributes.high_clearance == true
    not user.attributes.suspended
}

# Same department access with clearance match
allow := true if {
    user.attributes.department == resource.attributes.department
    user.attributes.clearance_level >= resource.attributes.clearance_level
    user.attributes.status == "active"
}

# High clearance users can access non-secret resources in their dept
allow := true if {
    user.attributes.high_clearance == true
    user.attributes.department == resource.attributes.department
    resource.attributes.classification != "secret"
    input.action == "read"
}

# Resource owners can always access (if active)
allow := true if {
    user.id == resource.attributes.owner_id
    user.attributes.status == "active"
}

# Public resources accessible to active users
allow := true if {
    resource.attributes.classification == "public"
    user.attributes.status == "active"
    input.action == "read"
}
