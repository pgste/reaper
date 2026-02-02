# String Operations Policy - OPA Equivalent
# Tests string methods matching Reaper string_policy.reap

package string_ops

import rego.v1

default allow := false

# Case insensitive match - lower()
allow if {
    user := data.entities[_]
    user.id == input.principal
    resource := data.entities[_]
    resource.id == input.resource
    resource.attributes.type == "case_insensitive"
    lower(user.attributes.name) == "john doe"
}

# Uppercase code check - upper()
allow if {
    user := data.entities[_]
    user.id == input.principal
    resource := data.entities[_]
    resource.id == input.resource
    resource.attributes.type == "code_entry"
    upper(user.attributes.access_code) == "ADMIN123"
}

# Trimmed role check - trim()
allow if {
    user := data.entities[_]
    user.id == input.principal
    resource := data.entities[_]
    resource.id == input.resource
    resource.attributes.type == "trimmed_check"
    trim_space(user.attributes.role) == "manager"
}

# Email contains domain - contains()
allow if {
    user := data.entities[_]
    user.id == input.principal
    resource := data.entities[_]
    resource.id == input.resource
    resource.attributes.type == "internal_docs"
    contains(user.attributes.email, "@company.com")
}

# Username prefix - startswith()
allow if {
    user := data.entities[_]
    user.id == input.principal
    resource := data.entities[_]
    resource.id == input.resource
    resource.attributes.type == "system_settings"
    startswith(user.attributes.username, "admin_")
}

# Email suffix - endswith()
allow if {
    user := data.entities[_]
    user.id == input.principal
    resource := data.entities[_]
    resource.id == input.resource
    resource.attributes.type == "classified_docs"
    endswith(user.attributes.email, ".gov")
}

# Full name split validation - split()
allow if {
    user := data.entities[_]
    user.id == input.principal
    resource := data.entities[_]
    resource.id == input.resource
    resource.attributes.type == "profile"
    name_parts := split(user.attributes.name, " ")
    count(name_parts) >= 2
}

# Complex email validation - chained operations
allow if {
    user := data.entities[_]
    user.id == input.principal
    resource := data.entities[_]
    resource.id == input.resource
    resource.attributes.type == "email_check"
    lower_email := lower(user.attributes.email)
    trimmed_email := trim_space(lower_email)
    contains(trimmed_email, "@")
    endswith(trimmed_email, ".com")
}
