package reaper.regex

import rego.v1

# Regex Validation Policy — mirrors regex_policy.reap exactly
# Rules:
#   1. valid_email, 2. valid_phone, 3. valid_url, 4. valid_ip,
#   5. valid_uuid, 6. valid_credit_card, 7. redacted_data,
#   8. csv_parsing, 9. log_analysis

default allow := false

# Entity lookups — the map value IS the attributes object (see deploy-opa.sh)
user := data.entities[input.principal]

resource := data.entities[input.resource]

# Email Validation
allow if {
    user.email != null
    resource.type == "email_validation"
    regex.match(`^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$`, user.email)
}

# Phone Number Validation (US format)
allow if {
    user.phone != null
    resource.type == "phone_validation"
    regex.match(`^\(\d{3}\) \d{3}-\d{4}$`, user.phone)
}

# URL Validation
allow if {
    user.url != null
    resource.type == "url_validation"
    regex.match(`^https?://[a-zA-Z0-9.-]+`, user.url)
}

# IP Address Validation (IPv4)
allow if {
    user.ip_address != null
    resource.type == "ip_validation"
    regex.match(`^((25[0-5]|(2[0-4]|1\d|[1-9]|)\d)\.?){4}$`, user.ip_address)
}

# UUID Validation
allow if {
    user.uuid != null
    resource.type == "uuid_validation"
    regex.match(`^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$`, user.uuid)
}

# Credit Card Number Validation (basic format check)
allow if {
    user.credit_card != null
    resource.type == "payment_validation"
    regex.match(`^\d{4}-\d{4}-\d{4}-\d{4}$`, user.credit_card)
}

# Redacted Data Rule (SSN redaction check)
allow if {
    user.has_redacted_ssn == true
    resource.type == "redacted_data"
}

# CSV Parsing Rule
allow if {
    user.has_valid_csv == true
    resource.type == "csv_data"
}

# Log Entry Analysis Rule
allow if {
    user.has_valid_log == true
    resource.type == "log_entry"
}
