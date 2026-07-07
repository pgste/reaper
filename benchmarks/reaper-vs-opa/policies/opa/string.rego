package reaper.string

import rego.v1

# String Operations Policy — mirrors string_policy.reap exactly
# Rules:
#   1. case_insensitive_match: lower(name) == "john doe"
#   2. uppercase_code_check: upper(access_code) == "ADMIN123"
#   3. trimmed_role_check: trim(role) == "manager"
#   4. email_contains_domain: email contains "@company.com"
#   5. username_prefix: username starts with "admin_"
#   6. email_suffix: email ends with ".gov"
#   7. full_name_validation: name split by space has >= 2 parts
#   8. complex_email_validation: lower+trim email contains "@" and ends ".com"

default allow := false

# Entity lookups — the map value IS the attributes object (see deploy-opa.sh)
user := data.entities[input.principal]

resource := data.entities[input.resource]

# Lowercase comparison
allow if {
    user.name != null
    resource.type == "case_insensitive"
    lower(user.name) == "john doe"
}

# Uppercase code validation
allow if {
    user.access_code != null
    resource.type == "code_entry"
    upper(user.access_code) == "ADMIN123"
}

# Trimmed string comparison
allow if {
    user.role != null
    resource.type == "trimmed_check"
    trim_space(user.role) == "manager"
}

# String contains check
allow if {
    user.email != null
    resource.type == "internal_docs"
    contains(user.email, "@company.com")
}

# String starts with check
allow if {
    user.username != null
    resource.type == "system_settings"
    startswith(user.username, "admin_")
}

# String ends with check
allow if {
    user.email != null
    resource.type == "classified_docs"
    endswith(user.email, ".gov")
}

# String split check — name has at least 2 parts
allow if {
    user.name != null
    resource.type == "profile"
    count(split(user.name, " ")) >= 2
}

# Complex string operations
allow if {
    user.email != null
    resource.type == "email_check"
    trimmed := trim_space(lower(user.email))
    contains(trimmed, "@")
    endswith(trimmed, ".com")
}
