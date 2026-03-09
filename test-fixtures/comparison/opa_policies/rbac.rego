# RBAC Policy - OPA Equivalent
# Tests role-based access control matching Reaper rbac.reap policy

package rbac

import rego.v1

default allow := false

# Admin has full access
allow if {
    user := data.entities[_]
    user.id == input.principal
    user.attributes.role == "admin"
}

# Manager can read and write
allow if {
    user := data.entities[_]
    user.id == input.principal
    user.attributes.role == "manager"
    input.action in ["read", "write"]
}

# Engineer can read and write
allow if {
    user := data.entities[_]
    user.id == input.principal
    user.attributes.role == "engineer"
    input.action in ["read", "write"]
}

# Viewer can only read
allow if {
    user := data.entities[_]
    user.id == input.principal
    user.attributes.role == "viewer"
    input.action == "read"
}

# User can access owned resources
allow if {
    user := data.entities[_]
    user.id == input.principal
    resource := data.entities[_]
    resource.id == input.resource
    resource.attributes.owner == input.principal
}
