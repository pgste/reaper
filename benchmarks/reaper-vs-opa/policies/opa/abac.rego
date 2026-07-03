package reaper.abac

import rego.v1

# ABAC Policy — mirrors policies/reaper/abac.reap (policy abac_benchmark) exactly.
#
# Reaper rules (default: deny; an explicit deny wins over any allow):
#   deny_suspended:       deny  if user.suspended == true
#   admin_high_clearance: allow if user.role == "admin" && user.high_clearance == true
#   department_clearance: allow if user.department == resource.department
#                                  && user.clearance_level >= resource.clearance_level
#                                  && user.status == "active"
#   high_clearance_dept:  allow if user.high_clearance == true
#                                  && user.department == resource.department
#                                  && resource.classification != "secret"
#                                  && action == "read"
#   owner_access:         allow if user.id == resource.owner_id && user.status == "active"
#   public_access:        allow if resource.classification == "public"
#                                  && user.status == "active" && action == "read"
#
# Input shape (uniform across scenarios): {principal: <id>, action, resource, context}.
# The entity map value IS the attributes object (see deploy-opa.sh). `not deny`
# on every allow rule encodes Reaper's deny-precedence.

default allow := false

user := data.entities[input.principal]

resource := data.entities[input.resource]

# deny_suspended (highest priority)
deny if {
	user.suspended == true
}

# admin_high_clearance
allow if {
	not deny
	user.role == "admin"
	user.high_clearance == true
}

# department_clearance
allow if {
	not deny
	user.department == resource.department
	user.clearance_level >= resource.clearance_level
	user.status == "active"
}

# high_clearance_dept
allow if {
	not deny
	user.high_clearance == true
	user.department == resource.department
	resource.classification != "secret"
	input.action == "read"
}

# owner_access
allow if {
	not deny
	user.id == resource.owner_id
	user.status == "active"
}

# public_access
allow if {
	not deny
	resource.classification == "public"
	user.status == "active"
	input.action == "read"
}
