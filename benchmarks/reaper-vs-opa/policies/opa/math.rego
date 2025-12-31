package reaper.math

import rego.v1

# Default deny
default allow := false

# Entity lookup from pre-loaded data
user := data.entities[input.principal]

# Credit Score Threshold
allow if {
    user.credit_score != null
    input.resource == "premium_loan"
    user.credit_score >= 700
}

# Budget Limit Check
allow if {
    user.order_total != null
    user.budget_limit != null
    input.resource == "shopping_cart"
    user.order_total <= user.budget_limit
}

# Average Rating Check
allow if {
    user.average_rating != null
    input.resource == "featured_listing"
    user.average_rating >= 4.0
}

# Price Range Check
allow if {
    user.list_price != null
    input.resource == "marketplace"
    user.list_price >= 1
    user.list_price <= 10000
}

# Tier Upgrade Check
allow if {
    user.score != null
    input.resource == "premium_tier"
    user.score >= 90.0
}

# Temperature Range Check
allow if {
    user.temperature != null
    input.resource == "temperature_monitor"
    user.temperature >= -50
    user.temperature <= 50
}

# Loyalty Points Check
allow if {
    user.total_points != null
    input.resource == "loyalty_reward"
    user.total_points >= 1000
}

# Discount Percentage Check
allow if {
    user.discount_percentage != null
    input.resource == "sale_item"
    user.discount_percentage >= 0
    user.discount_percentage <= 50
}
