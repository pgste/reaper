package reaper.rebac

import future.keywords.if
import future.keywords.in

# Default deny
default allow := false

# Direct O(1) entity lookups (entities pre-indexed as map)
user := data.entities[input.principal]
resource := data.entities[input.resource]

# Owners have full access to their resources
allow := true if {
    user.id == resource.attributes.owner_id
}

# Team members can access team resources (if not pending)
allow := true if {
    user.attributes.team_id == resource.attributes.team_id
    user.attributes.team_role != "pending"
    resource.attributes.team_accessible == true
}

# Team leads have full access to team resources
allow := true if {
    user.attributes.team_role == "lead"
    user.attributes.team_id == resource.attributes.team_id
}

# Shared resources - users in share list can access
allow := true if {
    user.id == resource.attributes.shared_with
    resource.attributes.share_active == true
}

# Active collaborators can edit (but not delete)
allow := true if {
    user.id == resource.attributes.collaborator_id
    resource.attributes.collaboration_status == "active"
    input.action != "delete"
}

# Parent-child relationship - access parent's resources (read only)
allow := true if {
    user.id == resource.attributes.parent_owner
    resource.attributes.inherit_permissions == true
    input.action == "read"
}

# Senior managers can access subordinate resources in their dept (read only)
allow := true if {
    user.attributes.is_senior_manager == true
    user.attributes.department == resource.attributes.department
    resource.attributes.visible_to_managers == true
    input.action == "read"
}

# Group members can access group resources
allow := true if {
    user.attributes.group_id == resource.attributes.group_id
    user.attributes.group_member == true
    resource.attributes.group_accessible == true
}

# Delegates can access on behalf of others
allow := true if {
    user.id == resource.attributes.delegated_to
    resource.attributes.delegation_active == true
}
