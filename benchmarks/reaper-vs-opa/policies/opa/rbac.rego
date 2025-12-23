package rbac

import rego.v1

# Default deny
default allow := false

# Direct O(1) entity lookup (entities pre-indexed as map)
user := data.entities[input.principal]

# Admin has full access
allow if {
    user.attributes.role == "admin"
}

# Manager can read and write
allow if {
    user.attributes.role == "manager"
    input.action in ["read", "write"]
}

# Engineer can read and write in engineering resources
allow if {
    user.attributes.role == "engineer"
    input.action in ["read", "write"]
    startswith(input.resource, "/api/engineering/")
}

# Viewer can only read
allow if {
    user.attributes.role == "viewer"
    input.action == "read"
}

# User can read public resources
allow if {
    input.action == "read"
    startswith(input.resource, "/api/public/")
}
