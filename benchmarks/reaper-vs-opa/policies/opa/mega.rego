package reaper.mega

import rego.v1

# Mega Policy with 50+ Rules — mirrors mega.reap from benchmarks
# Uses only features supported by both Reaper and OPA for fair comparison

# Default deny
default allow := false

# Entity lookup — .attributes shorthand
user := data.entities[input.principal.id].attributes

# ===== MATH OPERATIONS (Rules 1-15) =====

# Rule 1: Credit Score Check
allow if {
    input.resource == "premium_loan"
    user.credit_score >= 700
}

# Rule 2: Budget Limit Check
allow if {
    user.order_total != null
    user.budget_limit != null
    input.resource == "shopping_cart"
    user.order_total <= user.budget_limit
}

# Rule 3: Average Rating Check
allow if {
    input.resource == "featured_listing"
    user.average_rating >= 4.0
}

# Rule 4: Price Range Check - Low
allow if {
    input.resource == "marketplace_low"
    user.list_price >= 1
    user.list_price <= 100
}

# Rule 5: Price Range Check - Medium
allow if {
    input.resource == "marketplace_medium"
    user.list_price >= 100
    user.list_price <= 1000
}

# Rule 6: Price Range Check - High
allow if {
    input.resource == "marketplace_high"
    user.list_price >= 1000
    user.list_price <= 10000
}

# Rule 7: Tier Upgrade - Bronze
allow if {
    input.resource == "bronze_tier"
    user.score >= 50.0
    user.score < 70.0
}

# Rule 8: Tier Upgrade - Silver
allow if {
    input.resource == "silver_tier"
    user.score >= 70.0
    user.score < 90.0
}

# Rule 9: Tier Upgrade - Gold
allow if {
    input.resource == "gold_tier"
    user.score >= 90.0
}

# Rule 10: Temperature Range - Cold
allow if {
    input.resource == "cold_storage"
    user.temperature >= -50
    user.temperature <= 0
}

# Rule 11: Temperature Range - Normal
allow if {
    input.resource == "normal_storage"
    user.temperature >= 0
    user.temperature <= 25
}

# Rule 12: Temperature Range - Warm
allow if {
    input.resource == "warm_storage"
    user.temperature >= 25
    user.temperature <= 50
}

# Rule 13: Loyalty Points - Basic
allow if {
    input.resource == "loyalty_basic"
    user.total_points >= 100
    user.total_points < 500
}

# Rule 14: Loyalty Points - Premium
allow if {
    input.resource == "loyalty_premium"
    user.total_points >= 500
    user.total_points < 1000
}

# Rule 15: Loyalty Points - Elite
allow if {
    input.resource == "loyalty_elite"
    user.total_points >= 1000
}

# ===== STRING OPERATIONS (Rules 22-30) =====

# Rule 22: Email Contains Domain - Company
allow if {
    input.resource == "company_docs"
    contains(user.email, "@company.com")
}

# Rule 23: Email Contains Domain - Partner
allow if {
    input.resource == "partner_docs"
    contains(user.email, "@partner.com")
}

# Rule 24: Email Contains Domain - External
allow if {
    input.resource == "external_docs"
    contains(user.email, "@external.com")
}

# Rule 25: Username Prefix - Admin
allow if {
    input.resource == "admin_panel"
    startswith(user.username, "admin_")
}

# Rule 26: Username Prefix - Manager
allow if {
    input.resource == "manager_panel"
    startswith(user.username, "mgr_")
}

# Rule 27: Username Prefix - User
allow if {
    input.resource == "user_panel"
    startswith(user.username, "user_")
}

# Rule 28: Email Suffix - Gov
allow if {
    input.resource == "gov_classified"
    endswith(user.email, ".gov")
}

# Rule 29: Email Suffix - Mil
allow if {
    input.resource == "mil_classified"
    endswith(user.email, ".mil")
}

# Rule 30: Email Suffix - Edu
allow if {
    input.resource == "edu_resources"
    endswith(user.email, ".edu")
}

# ===== REGEX VALIDATION (Rules 31-45) =====

# Rule 31: Email Validation
allow if {
    input.resource == "email_validation"
    regex.match(`^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$`, user.email)
}

# Rule 32: Phone Validation
allow if {
    input.resource == "phone_validation"
    regex.match(`^\+?1?\s?\(?\d{3}\)?[\s.-]?\d{3}[\s.-]?\d{4}$`, user.phone)
}

# Rule 33: URL Validation
allow if {
    input.resource == "url_validation"
    regex.match(`^https?://[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}(/.*)?$`, user.url)
}

# Rule 34: IP Validation
allow if {
    input.resource == "ip_validation"
    regex.match(`^((25[0-5]|(2[0-4]|1\d|[1-9]|)\d)\.?){4}$`, user.ip_address)
}

# Rule 35: UUID Validation
allow if {
    input.resource == "uuid_validation"
    regex.match(`^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$`, user.uuid)
}

# Rule 36: Credit Card Validation
allow if {
    input.resource == "payment_validation"
    regex.match(`^\d{4}[\s-]?\d{4}[\s-]?\d{4}[\s-]?\d{4}$`, user.credit_card)
}

# Rule 37: Zip Validation
allow if {
    input.resource == "zip_validation"
    regex.match(`^\d{5}(-\d{4})?$`, user.zip_code)
}

# Rule 38: Color Validation
allow if {
    input.resource == "color_validation"
    regex.match(`^#[0-9A-Fa-f]{6}$`, user.hex_color)
}

# Rule 39: Date Validation
allow if {
    input.resource == "date_validation"
    regex.match(`^\d{4}-\d{2}-\d{2}$`, user.date_iso)
}

# Rule 40: Time Validation
allow if {
    input.resource == "time_validation"
    regex.match(`^([01]\d|2[0-3]):[0-5]\d$`, user.time_24h)
}

# Rule 41: Domain Validation
allow if {
    input.resource == "domain_validation"
    regex.match(`^([a-zA-Z0-9-]+\.)+[a-zA-Z]{2,}$`, user.domain_name)
}

# Rule 42: Username Validation
allow if {
    input.resource == "username_validation"
    regex.match(`^[a-zA-Z0-9_-]{3,16}$`, user.username_pattern)
}

# Rule 43: MAC Validation
allow if {
    input.resource == "mac_validation"
    regex.match(`^([0-9A-Fa-f]{2}[:-]){5}([0-9A-Fa-f]{2})$`, user.mac_address)
}

# Rule 44: IPv6 Validation
allow if {
    input.resource == "ipv6_validation"
    regex.match(`^([0-9a-fA-F]{1,4}:){7}[0-9a-fA-F]{1,4}$`, user.ipv6_address)
}

# Rule 45: Version Validation
allow if {
    input.resource == "version_validation"
    regex.match(`^\d+\.\d+\.\d+$`, user.semver)
}

# ===== TIME OPERATIONS (Rules 46-60) =====

# Rule 46: Token Expiration
allow if {
    input.resource == "api_endpoint"
    user.token_expires_at > 1765180000000000000
}

# Rule 47: Business Hours - Morning
allow if {
    user.role == "employee"
    input.resource == "office_morning"
    user.work_start_time < 1765185000000000000
}

# Rule 48: Business Hours - Afternoon
allow if {
    user.role == "employee"
    input.resource == "office_afternoon"
    user.work_end_time > 1765185000000000000
}

# Rule 49: Age Verification - 18+
allow if {
    input.resource == "adult_content"
    user.birthdate < 1100000000
}

# Rule 50: Age Verification - 21+
allow if {
    input.resource == "alcohol_purchase"
    user.birthdate < 1000000000
}

# Rule 51: Lease Active
allow if {
    input.resource == "apartment_access"
    user.lease_end_time > 1765180000000000000
}

# Rule 54: Session Valid
allow if {
    input.resource == "web_session"
    user.session_start_time < 1765185000000000000
}

# Rule 55: Future Event
allow if {
    user.role == "event_planner"
    input.resource == "conference_room"
    user.event_scheduled_time > 1765180000000000000
}

# Rule 56: Subscription Active
allow if {
    input.resource == "premium_feature"
    user.subscription_expires > 1765180000000000000
}

# Rule 57: Trial Active
allow if {
    input.resource == "trial_feature"
    user.trial_ends > 1765180000000000000
}

# Rule 58: Contract Started
allow if {
    input.resource == "contractor_tools"
    user.contract_start < 1765185000000000000
}

# Rule 59: Contract Active
allow if {
    input.resource == "contractor_data"
    user.contract_end > 1765185000000000000
}

# Rule 60: Certification Valid
allow if {
    input.resource == "certified_operation"
    user.certification_expires > 1765180000000000000
}

# ===== COLLECTION OPERATIONS (Rules 61-66) =====

# Rule 61: Array Contains - Read
allow if {
    input.resource == "doc_read"
    "read" in user.permissions
}

# Rule 62: Array Contains - Write
allow if {
    input.resource == "doc_write"
    "write" in user.permissions
}

# Rule 63: Array Contains - Delete
allow if {
    input.resource == "doc_delete"
    "delete" in user.permissions
}

# Rule 64: Array Length - Junior
allow if {
    input.resource == "junior_position"
    count(user.skills) >= 2
}

# Rule 65: Array Length - Mid
allow if {
    input.resource == "mid_position"
    count(user.skills) >= 3
}

# Rule 66: Array Length - Senior
allow if {
    input.resource == "senior_position"
    count(user.skills) >= 5
}

# ===== JSON FIELD VALIDATION =====

# Rule 101: Valid Number Field
allow if {
    input.resource == "age_verification"
    user.age >= 18
}

# Rule 102: Valid Boolean Field
allow if {
    input.resource == "verification_check"
    user.verified == true
}
