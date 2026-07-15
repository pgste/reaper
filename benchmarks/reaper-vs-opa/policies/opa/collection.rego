# Collection operations policy.
#
# MUST stay decision-equivalent to policies/reaper/collection.reap — the
# benchmark enforces cross-engine parity per request before measuring.
# Exactly the reap policy's eight rules: document read/write/admin
# permission membership (NO action gating — reap has none), senior_position
# skill count, shared_resource group membership (engineering / platform /
# admin), and system admin role. No extra resource types.

package reaper.collection

import rego.v1

default allow := false

user := data.entities[input.principal]

resource := data.entities[input.resource]

# has_read_permission
allow if {
	resource.type == "document"
	"read" in user.permissions
}

# has_write_permission
allow if {
	resource.type == "document"
	"write" in user.permissions
}

# has_admin_permission
allow if {
	resource.type == "document"
	"admin" in user.permissions
}

# minimum_skills
allow if {
	resource.type == "senior_position"
	count(user.skills) >= 5
}

# engineering_access
allow if {
	resource.type == "shared_resource"
	"engineering" in user.groups
}

# platform_access
allow if {
	resource.type == "shared_resource"
	"platform" in user.groups
}

# admin_group_access
allow if {
	resource.type == "shared_resource"
	"admin" in user.groups
}

# has_admin_role
allow if {
	resource.type == "system"
	"admin" in user.roles
}
