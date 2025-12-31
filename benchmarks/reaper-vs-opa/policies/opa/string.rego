package reaper.string

import rego.v1

# Default deny
default allow := false

# Entity lookup from pre-loaded data
user := data.entities[input.principal]

# Email domain check - uses endswith
allow if {
    input.resource == "internal_docs"
    endswith(user.email, "@company.com")
}

# Partner email check - uses contains
allow if {
    input.resource == "partner_portal"
    contains(user.email, "partner")
}

# Admin username check - uses startswith
allow if {
    input.resource == "admin_panel"
    startswith(user.username, "admin_")
}

# Government email check - uses endswith
allow if {
    input.resource == "gov_service"
    endswith(user.email, ".gov")
}

# Test username check - uses contains
allow if {
    input.resource == "test_environment"
    contains(user.username, "test")
}
