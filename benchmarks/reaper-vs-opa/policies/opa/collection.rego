package reaper.collection

import rego.v1

# Default deny
default allow := false

# Entity lookup from pre-loaded data
user := data.entities[input.principal]

# Array Contains Check - check if user has read permission
allow if {
    input.resource == "document"
    "read" in user.permissions
}

# Check for write permission
allow if {
    input.resource == "document"
    "write" in user.permissions
}

# Check for admin permission
allow if {
    input.resource == "document"
    "admin" in user.permissions
}

# Array Length Check - minimum 5 skills for senior position
allow if {
    input.resource == "senior_position"
    count(user.skills) >= 5
}

# Check if user is in engineering group
allow if {
    input.resource == "shared_resource"
    "engineering" in user.groups
}

# Check if user is in platform group
allow if {
    input.resource == "shared_resource"
    "platform" in user.groups
}

# Check if user is in admin group
allow if {
    input.resource == "shared_resource"
    "admin" in user.groups
}

# Check if user is admin
allow if {
    input.resource == "system"
    "admin" in user.roles
}
