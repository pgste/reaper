package reaper.rbac

import rego.v1

# Default deny
default allow := false

# Admin has full access
allow if {
    input.principal.role == "admin"
}

# Manager can read and write
allow if {
    input.principal.role == "manager"
    input.action in ["read", "write"]
}

# Engineer can read and write in engineering resources
allow if {
    input.principal.role == "engineer"
    input.action in ["read", "write"]
    startswith(input.resource, "/api/engineering/")
}

# Viewer can only read
allow if {
    input.principal.role == "viewer"
    input.action == "read"
}

# User can read public resources
allow if {
    input.action == "read"
    startswith(input.resource, "/api/public/")
}
