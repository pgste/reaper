package reaper.json

import rego.v1

# Default deny
default allow := false

# Entity lookup from pre-loaded data
user := data.entities[input.principal]

# JSON Parse Check
allow if {
    user.payload != null
    user.payload.valid == true
    input.resource == "api_endpoint"
}

# JSON Path Access Check
allow if {
    user.profile != null
    user.profile.name != null
    user.profile.email != null
    user.profile.phone != null
    user.profile.address != null
    input.resource == "user_profile"
}

# Nested JSON Structure Check
allow if {
    user.payment != null
    user.payment.card != null
    user.payment.card.number != null
    user.payment.billing_address != null
    user.payment.billing_address.street != null
    input.resource == "payment"
}

# JSON Array Check
allow if {
    user.order_items != null
    count(user.order_items) > 0
    input.resource == "order"
}

# JSON Type Check
allow if {
    user.form_data != null
    user.form_data.name != null
    user.form_data.age != null
    user.form_data.active != null
    input.resource == "form_data"
    is_string(user.form_data.name)
    is_number(user.form_data.age)
    is_boolean(user.form_data.active)
}

# JSON String Field
allow if {
    user.name != null
    input.resource == "text_field"
    user.name_length > 0
}

# JSON Number Field
allow if {
    user.age != null
    input.resource == "number_field"
    user.age >= 18
}

# JSON Boolean Field
allow if {
    user.verified != null
    input.resource == "boolean_field"
    user.verified == true
}

# JSON Object Field
allow if {
    user.address != null
    user.address.street != null
    user.address.city != null
    input.resource == "structured_data"
}

# JSON Merge Check
allow if {
    user.primary_data != null
    user.primary_data.name != null
    user.primary_data.email != null
    user.secondary_data != null
    user.secondary_data.phone != null
    user.secondary_data.address != null
    input.resource == "data_merge"
}
