# Multilayer Access Control — RBAC + ABAC + ReBAC combined.
#
# MUST stay decision-equivalent to policies/reaper/multilayer.reap — the
# benchmark enforces cross-engine parity per request before measuring, so
# every rule below mirrors its .reap counterpart condition-for-condition
# (same attribute names, same clearance comparisons, same action gates).
#   Deny: deny_suspended, deny_intern_classified
#   Allow: admin_full_access, owner_with_clearance, team_lead_access,
#          team_senior_with_clearance, department_clearance_match,
#          collaborator_access, shared_resource_access, executive_access,
#          manager_hierarchy_access, public_resource_access

package reaper.multilayer

import rego.v1

default allow := false

user := data.entities[input.principal]

resource := data.entities[input.resource]

# deny_suspended
deny if {
	user.suspended == true
}

# deny_intern_classified
deny if {
	user.role == "intern"
	resource.classification == "secret"
}

# admin_full_access: admin AND high clearance (reap requires both)
allow if {
	not deny
	user.role == "admin"
	user.high_clearance == true
}

# owner_with_clearance: ownership + clearance level + active status
allow if {
	not deny
	user.id == resource.owner_id
	user.clearance_level >= resource.clearance_level
	user.status == "active"
}

# team_lead_access
allow if {
	not deny
	user.team_role == "lead"
	user.team_id == resource.team_id
}

# team_senior_with_clearance
allow if {
	not deny
	user.team_id == resource.team_id
	user.role == "senior"
	user.clearance_level >= resource.clearance_level
	resource.team_accessible == true
}

# department_clearance_match
allow if {
	not deny
	user.department == resource.department
	user.clearance_level >= resource.clearance_level
	user.status == "active"
	resource.classification != "secret"
}

# collaborator_access (no deletes)
allow if {
	not deny
	user.id == resource.collaborator_id
	resource.collaboration_status == "active"
	input.action != "delete"
}

# shared_resource_access
allow if {
	not deny
	user.id == resource.shared_with
	resource.share_active == true
}

# executive_access
allow if {
	not deny
	user.role == "executive"
	user.high_clearance == true
	resource.classification != "secret"
	user.status == "active"
}

# manager_hierarchy_access (read only)
allow if {
	not deny
	user.is_senior_manager == true
	user.department == resource.department
	resource.visible_to_managers == true
	input.action == "read"
}

# public_resource_access (read only)
allow if {
	not deny
	resource.classification == "public"
	user.status == "active"
	input.action == "read"
}
