package reaper.rbac

import rego.v1

# RBAC Policy — mirrors policies/reaper/rbac.reap (policy rbac_benchmark) exactly.
#
# Reaper rules (default: deny):
#   admin_full_access:  user.role == "admin"
#   manager_read:       user.role == "manager" && action == "read"
#   manager_write:      user.role == "manager" && action == "write"
#   engineer_read:      user.role == "engineer" && action == "read"
#   engineer_write:     user.role == "engineer" && action == "write"
#   viewer_read_only:   user.role == "viewer"  && action == "read"
#
# Input shape (uniform across scenarios): {principal: <id>, action, resource, context}.
# The entity map value IS the attributes object (see deploy-opa.sh), so the role
# is read as user.role — mirroring Reaper's attribute-based resolution.

default allow := false

user := data.entities[input.principal]

# admin_full_access
allow if {
	user.role == "admin"
}

# manager_read
allow if {
	user.role == "manager"
	input.action == "read"
}

# manager_write
allow if {
	user.role == "manager"
	input.action == "write"
}

# engineer_read
allow if {
	user.role == "engineer"
	input.action == "read"
}

# engineer_write
allow if {
	user.role == "engineer"
	input.action == "write"
}

# viewer_read_only
allow if {
	user.role == "viewer"
	input.action == "read"
}
