package reaper.multilayer

import rego.v1

# Multilayer Enterprise Policy — mirrors multilayer_enterprise.reap exactly
# 9 layers combining RBAC, ABAC, and ReBAC with deny precedence
# Rules:
#   Deny: deny_suspended, deny_intern_classified
#   Allow: admin, owner+clearance, team_lead, team_senior, dept+clearance,
#          shared, collaborator, executive, manager_hierarchy, public

default allow := false

# Entity lookups — .attributes shorthand
user := data.entities[input.principal.id].attributes

resource := data.entities[input.resource].attributes

# ===== DENY RULES (highest priority) =====

# Suspended users are blocked regardless of other factors
deny if {
    user.suspended == true
}

# Interns cannot access classified documents
deny if {
    user.role == "intern"
    resource.classification == "secret"
}

# ===== Layer 2: RBAC - Admin Override =====

# Admins have full access (except suspended)
allow if {
    not deny
    user.role == "admin"
}

# ===== Layer 3: ReBAC + ABAC - Ownership with Clearance =====

# Owners can access if they have clearance
allow if {
    not deny
    user.id == resource.owner_id
    user.high_clearance == true
    resource.archived != true
}

# ===== Layer 4: ReBAC + RBAC - Team Access with Role =====

# Team leads can access all team resources
allow if {
    not deny
    user.team_role == "lead"
    user.team_id == resource.team_id
}

# Team members can access if manager or senior
allow if {
    not deny
    user.team_id == resource.team_id
    user.role == "manager"
    user.team_role != "pending"
}

# ===== Layer 5: ABAC + ReBAC - Department with Clearance =====

# Same department access with clearance match
allow if {
    not deny
    user.department == resource.department
    user.clearance_match == true
    resource.archived != true
    resource.classification != "secret"
}

# ===== Layer 6: ReBAC - Sharing and Collaboration =====

# Shared resources
allow if {
    not deny
    user.id == resource.shared_with_user
}

# Active collaborators
allow if {
    not deny
    user.id == resource.collaborator_id
    resource.collaboration_status == "active"
}

# ===== Layer 7: RBAC + ABAC - Executive Access =====

# Executives with high clearance can access most things
allow if {
    not deny
    user.role == "executive"
    user.high_clearance == true
    resource.archived != true
}

# ===== Layer 8: ReBAC - Hierarchical Access =====

# Senior managers can access subordinate resources
allow if {
    not deny
    user.is_senior_manager == true
    user.department == resource.owner_department
    resource.public_in_dept == true
}

# ===== Layer 9: ABAC - Public Resources =====

# Public resources accessible to anyone active
allow if {
    not deny
    resource.classification == "public"
    user.status == "active"
    resource.archived != true
}
