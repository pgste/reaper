package reaper.regex

import rego.v1

# Default deny
default allow := false

# Entity lookup from pre-loaded data
user := data.entities[input.principal]

# Email Validation Rule - matches valid email format
allow if {
    input.resource == "email_service"
    regex.match("^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\\.[a-zA-Z]{2,}$", user.email)
}

# Phone Validation Rule - matches US phone format (555) 123-4567
allow if {
    input.resource == "phone_service"
    regex.match("^\\(\\d{3}\\) \\d{3}-\\d{4}$", user.phone)
}

# UUID Validation Rule - matches UUID format
allow if {
    input.resource == "uuid_service"
    regex.match("^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", user.uuid)
}

# Credit Card Validation Rule - matches credit card format XXXX-XXXX-XXXX-XXXX
allow if {
    input.resource == "payment_service"
    regex.match("^\\d{4}-\\d{4}-\\d{4}-\\d{4}$", user.credit_card)
}

# URL Validation Rule - matches URL format
allow if {
    input.resource == "web_service"
    regex.match("^https?://[a-zA-Z0-9.-]+", user.url)
}
