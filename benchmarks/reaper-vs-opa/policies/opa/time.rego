package reaper.time

import rego.v1

# Default deny
default allow := false

# Entity lookup from pre-loaded data
user := data.entities[input.principal]

# Token Expiration Policy
allow if {
    user.token_expires_at != null
    input.resource == "api_endpoint"
    user.token_expires_at > 1765180000000000000
}

# Business Hours Policy (9 AM - 5 PM)
allow if {
    user.role == "employee"
    input.resource == "office_system"
    user.work_start_time != null
    user.work_end_time != null
    user.work_start_time < 1765185000000000000
    user.work_end_time > 1765185000000000000
}

# Age Verification Policy (21+)
allow if {
    user.birthdate != null
    input.context.requires_age_verification == true
    user.birthdate < 1000000000
}

# Lease Expiration Policy
allow if {
    user.lease_end_time != null
    input.resource == "apartment"
    user.lease_end_time > 1765180000000000000
}

# Maintenance Window Policy
allow if {
    user.role == "operator"
    input.resource == "production_system"
    input.context.maintenance_window_start != null
    input.context.maintenance_window_end != null
    input.context.maintenance_window_start < 1765185000000000000
    input.context.maintenance_window_end > 1765185000000000000
}

# Session Extension
allow if {
    user.session_start_time != null
    user.session_extension_ns != null
    input.resource == "web_session"
    user.session_start_time < 1765185000000000000
}

# Future Event Scheduling
allow if {
    user.role == "event_planner"
    input.resource == "conference_room"
    user.event_scheduled_time != null
    user.event_scheduled_time > 1765180000000000000
}

# Temporary Access Grant
allow if {
    user.role == "contractor"
    input.resource == "project_files"
    user.access_grant_start != null
    user.access_grant_end != null
    user.access_grant_start < 1765185000000000000
    user.access_grant_end > 1765185000000000000
}

# RFC3339 Timestamp Validation
allow if {
    user.role == "system"
    input.resource == "timestamp_data"
    user.has_valid_timestamp == true
}

# Audit Logging
allow if {
    user.role == "audit_logger"
    input.resource == "audit_trail"
    user.can_log_audit == true
}

# Rate Limiting
allow if {
    user.role == "api_client"
    input.resource == "rate_limited_endpoint"
    user.last_request_time != null
    user.rate_limit_window_ns != null
    user.last_request_time < 1765186000000000000
}

# Data Retention Policy
allow if {
    user.role == "archiver"
    input.resource == "data"
    input.context.creation_time != null
    input.context.retention_period_ns != null
    input.context.creation_time >= 1765180000000000000
}
