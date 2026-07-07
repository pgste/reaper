package reaper.collection

import rego.v1

# Collection Operations Policy — mirrors collection_policy.reap exactly
# Rules:
#   1. array_contains_permission: check action-specific permissions
#   2. minimum_skills: skills count >= 5
#   3. group_overlap: user groups intersect with allowed groups
#   4. allowed_tags_only: all tags in allowed set
#   5. has_any_admin_role: "admin" in roles
#   6. all_projects_active: no inactive projects
#   7. required_metadata_keys: has name, email, phone keys
#   8. verified_emails_only: no unverified emails
#   9. nested_permissions: some dept has "execute" permission

default allow := false

# Entity lookups — the map value IS the attributes object (see deploy-opa.sh)
user := data.entities[input.principal]

resource := data.entities[input.resource]

# Array Contains Check - map actions to required permissions
allow if {
    user.permissions != null
    resource.type == "document"
    input.action == "view"
    "read" in user.permissions
}

allow if {
    user.permissions != null
    resource.type == "document"
    input.action == "edit"
    "write" in user.permissions
}

allow if {
    user.permissions != null
    resource.type == "document"
    "admin" in user.permissions
}

# Array Length Check — minimum 5 skills for senior position
allow if {
    user.skills != null
    resource.type == "senior_position"
    count(user.skills) >= 5
}

# Set Intersection Check (groups overlap with allowed groups)
allow if {
    user.groups != null
    resource.type == "shared_resource"
    overlap := {g | some g in user.groups; g in {"engineering", "platform", "admin"}}
    count(overlap) > 0
}

# Set Subset Check (all tags in allowed set)
allow if {
    user.tags != null
    resource.type == "content"
    allowed := {t | some t in user.tags; t in {"public", "draft", "review", "internal"}}
    count(allowed) == count(user.tags)
}

# Array Any Check (at least one admin role)
allow if {
    user.roles != null
    resource.type == "system"
    "admin" in user.roles
}

# Array All Check (all projects active — no inactive projects)
allow if {
    user.projects != null
    resource.type == "invoice"
    inactive := [p | some p in user.projects; p.active == false]
    count(inactive) == 0
}

# Map Keys Check (required metadata keys)
allow if {
    user.metadata != null
    resource.type == "profile"
    user.metadata.name
    user.metadata.email
    user.metadata.phone
}

# Comprehension Filter (all emails verified — no unverified)
allow if {
    user.email_addresses != null
    resource.type == "email_campaign"
    unverified := [e | some e in user.email_addresses; e.verified == false]
    count(unverified) == 0
}

# Nested Array Access (some department has execute permission)
allow if {
    user.departments != null
    resource.type == "workflow"
    exec_depts := [d | some d in user.departments; "execute" in d.permissions]
    count(exec_depts) > 0
}
