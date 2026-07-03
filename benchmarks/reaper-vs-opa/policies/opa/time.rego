package reaper.time

import rego.v1

# Time-Based Access Policy — mirrors time_policy.reap exactly
# Rules:
#   1. valid_token_access: token_expires_at > threshold
#   2. business_hours_access: work_start < threshold < work_end
#   3. age_verification: birthdate < threshold
#   4. active_lease_access: lease_end_time > threshold
#   5. maintenance_window_deployment: window brackets threshold
#   6. extended_session: session_start < threshold
#   7. future_event_scheduling: event_scheduled_time > threshold
#   8. temporary_access_grant: grant brackets threshold
#   9. timestamp_validation: has_valid_timestamp
#  10. audit_logging: can_log_audit
#  11. rate_limit_check: last_request_time < threshold
#  12. data_retention: creation_time >= threshold

default allow := false

# Entity lookups — the map value IS the attributes object (see deploy-opa.sh)
user := data.entities[input.principal]

resource := data.entities[input.resource]

# Token Expiration Policy (time::is_after → simple >)
allow if {
    user.token_expires_at != null
    resource.type == "api_endpoint"
    user.token_expires_at > 1765180000000000000
}

# Business Hours Policy (9 AM - 5 PM)
allow if {
    user.role == "employee"
    resource.type == "office_system"
    user.work_start_time != null
    user.work_end_time != null
    user.work_start_time < 1765185000000000000
    user.work_end_time > 1765185000000000000
}

# Age Verification Policy (21+)
allow if {
    user.birthdate != null
    resource.requires_age_verification == true
    user.birthdate < 1000000000
}

# Lease Expiration Policy
allow if {
    user.lease_end_time != null
    resource.type == "apartment"
    user.lease_end_time > 1765180000000000000
}

# Maintenance Window Policy
allow if {
    user.role == "operator"
    resource.type == "production_system"
    resource.maintenance_window_start != null
    resource.maintenance_window_end != null
    resource.maintenance_window_start < 1765185000000000000
    resource.maintenance_window_end > 1765185000000000000
}

# Session Extension
allow if {
    user.session_start_time != null
    user.session_extension_ns != null
    resource.type == "web_session"
    user.session_start_time < 1765185000000000000
}

# Future Event Scheduling
allow if {
    user.role == "event_planner"
    resource.type == "conference_room"
    user.event_scheduled_time != null
    user.event_scheduled_time > 1765180000000000000
}

# Temporary Access Grant
allow if {
    user.role == "contractor"
    resource.type == "project_files"
    user.access_grant_start != null
    user.access_grant_end != null
    user.access_grant_start < 1765185000000000000
    user.access_grant_end > 1765185000000000000
}

# RFC3339 Timestamp Validation
allow if {
    user.role == "system"
    resource.type == "timestamp_data"
    user.has_valid_timestamp == true
}

# Audit Logging
allow if {
    user.role == "audit_logger"
    resource.type == "audit_trail"
    user.can_log_audit == true
}

# Rate Limiting
allow if {
    user.role == "api_client"
    resource.type == "rate_limited_endpoint"
    user.last_request_time != null
    user.rate_limit_window_ns != null
    user.last_request_time < 1765186000000000000
}

# Data Retention Policy
allow if {
    user.role == "archiver"
    resource.type == "data"
    resource.creation_time != null
    resource.retention_period_ns != null
    resource.creation_time >= 1765180000000000000
}
