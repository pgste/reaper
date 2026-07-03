package reaper.rebac

import rego.v1

# ReBAC Policy — mirrors rebac_relationships.reap exactly
# Rules:
#   1. owner_full_access: user.id == resource.owner_id
#   2. team_member_access: same team && not pending
#   3. shared_access: user.id == resource.shared_with_user
#   4. parent_resource_access: user.id == resource.parent_owner_id && inherit_permissions
#   5. manager_subordinate_access: manager role && same dept && senior manager
#   6. collaborator_access: user.id == resource.collaborator_id && active
#   7. group_member_access: same group && group_member

default allow := false

# Entity lookups — the map value IS the attributes object (see deploy-opa.sh)
user := data.entities[input.principal]

resource := data.entities[input.resource]

# Owners have full access to their resources
allow if {
    user.id == resource.owner_id
}

# Team members can access team resources
allow if {
    user.team_id == resource.team_id
    user.team_role != "pending"
}

# Shared resources - users in share list can access
allow if {
    user.id == resource.shared_with_user
}

# Parent-child relationship - access parent's resources
allow if {
    user.id == resource.parent_owner_id
    resource.inherit_permissions == true
}

# Organization hierarchy - managers can access subordinate resources
allow if {
    user.role == "manager"
    user.department == resource.owner_department
    user.is_senior_manager == true
}

# Collaborators on specific resources
allow if {
    user.id == resource.collaborator_id
    resource.collaboration_status == "active"
}

# Group membership access
allow if {
    user.group_id == resource.group_id
    user.group_member == true
}
