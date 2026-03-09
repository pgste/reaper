package reaper.math

import rego.v1

# Math Operations Policy — mirrors math_policy.reap exactly
# Rules:
#   1. credit_score_check: >= 700
#   2. budget_check: order_total <= budget_limit
#   3. rating_check: average_rating >= 4.0
#   4. price_range_check: 1 <= list_price <= 10000
#   5. tier_upgrade: score >= 90.0
#   6. temperature_check: -50 <= temperature <= 50
#   7. loyalty_points: total_points >= 1000
#   8. discount_check: 0 <= discount_percentage <= 50

default allow := false

# Entity lookups — .attributes shorthand
user := data.entities[input.principal.id].attributes

resource := data.entities[input.resource].attributes

# Credit Score Threshold
allow if {
    user.credit_score != null
    resource.type == "premium_loan"
    user.credit_score >= 700
}

# Budget Limit Check
allow if {
    user.order_total != null
    user.budget_limit != null
    resource.type == "shopping_cart"
    user.order_total <= user.budget_limit
}

# Average Rating Check
allow if {
    user.average_rating != null
    resource.type == "featured_listing"
    user.average_rating >= 4.0
}

# Price Range Check
allow if {
    user.list_price != null
    resource.type == "marketplace"
    user.list_price >= 1
    user.list_price <= 10000
}

# Tier Upgrade Check
allow if {
    user.score != null
    resource.type == "premium_tier"
    user.score >= 90.0
}

# Temperature Range Check
allow if {
    user.temperature != null
    resource.type == "temperature_monitor"
    user.temperature >= -50
    user.temperature <= 50
}

# Loyalty Points Check
allow if {
    user.total_points != null
    resource.type == "loyalty_reward"
    user.total_points >= 1000
}

# Discount Percentage Check
allow if {
    user.discount_percentage != null
    resource.type == "sale_item"
    user.discount_percentage >= 0
    user.discount_percentage <= 50
}
