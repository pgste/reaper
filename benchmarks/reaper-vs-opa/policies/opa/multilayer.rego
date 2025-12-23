package reaper.multilayer

import future.keywords.if
import future.keywords.in

# Default deny
default allow := false

# Direct O(1) entity lookups (entities pre-indexed as map)
user := data.entities[input.principal]
resource := data.entities[input.resource]

# ===== DENY RULES (highest priority) =====

# Suspended users blocked
allow := false if {
    user.attributes.suspended == true
}

# Interns cannot access classified
allow := false if {
    user.attributes.role == "intern"
    resource.attributes.classification == "secret"
}

# ===== ADMIN OVERRIDE =====

# Admins with high clearance have full access
allow := true if {
    user.attributes.role == "admin"
    user.attributes.high_clearance == true
    not user.attributes.suspended
}

# ===== OWNERSHIP + CLEARANCE =====

# Owners can access if they have clearance
allow := true if {
    user.id == resource.attributes.owner_id
    user.attributes.clearance_level >= resource.attributes.clearance_level
    user.attributes.status == "active"
}

# ===== TEAM ACCESS + ROLE =====

# Team leads can access all team resources
allow := true if {
    user.attributes.team_role == "lead"
    user.attributes.team_id == resource.attributes.team_id
}

# Senior team members with clearance
allow := true if {
    user.attributes.team_id == resource.attributes.team_id
    user.attributes.role == "senior"
    user.attributes.clearance_level >= resource.attributes.clearance_level
    resource.attributes.team_accessible == true
}

# ===== DEPARTMENT + CLEARANCE =====

# Same department with clearance match
allow := true if {
    user.attributes.department == resource.attributes.department
    user.attributes.clearance_level >= resource.attributes.clearance_level
    user.attributes.status == "active"
    resource.attributes.classification != "secret"
}

# ===== COLLABORATION =====

# Active collaborators (but cannot delete)
allow := true if {
    user.id == resource.attributes.collaborator_id
    resource.attributes.collaboration_status == "active"
    input.action != "delete"
}

# Shared resources
allow := true if {
    user.id == resource.attributes.shared_with
    resource.attributes.share_active == true
}

# ===== EXECUTIVE ACCESS =====

# Executives with high clearance
allow := true if {
    user.attributes.role == "executive"
    user.attributes.high_clearance == true
    resource.attributes.classification != "secret"
    user.attributes.status == "active"
}

# ===== HIERARCHICAL ACCESS =====

# Senior managers can read dept resources
allow := true if {
    user.attributes.is_senior_manager == true
    user.attributes.department == resource.attributes.department
    resource.attributes.visible_to_managers == true
    input.action == "read"
}

# ===== PUBLIC ACCESS =====

# Public resources for active users
allow := true if {
    resource.attributes.classification == "public"
    user.attributes.status == "active"
    input.action == "read"
}
