package reaper.json

import rego.v1

# JSON Operations Policy — mirrors json_policy.reap exactly
# Rules:
#   1. valid_json_payload: payload.valid
#   2. complete_profile_data: profile has name/email/phone/address
#   3. nested_payment_data: payment.card.number + billing_address.street
#   4. order_items_present: order_items count > 0
#   5. correct_field_types: is_string, is_number, is_boolean
#   6. valid_string_field: name_length > 0
#   7. valid_number_field: age >= 18
#   8. valid_boolean_field: verified == true
#   9. valid_object_field: address.street + city
#  10. merged_data_complete: primary + secondary data

default allow := false

# Entity lookups — the map value IS the attributes object (see deploy-opa.sh)
user := data.entities[input.principal]

resource := data.entities[input.resource]

# JSON Parse Check
allow if {
    user.payload != null
    user.payload.valid == true
    resource.type == "api_endpoint"
}

# JSON Path Access Check
allow if {
    user.profile != null
    user.profile.name != null
    user.profile.email != null
    user.profile.phone != null
    user.profile.address != null
    resource.type == "user_profile"
}

# Nested JSON Structure Check
allow if {
    user.payment != null
    user.payment.card != null
    user.payment.card.number != null
    user.payment.billing_address != null
    user.payment.billing_address.street != null
    resource.type == "payment"
}

# JSON Array Check
allow if {
    user.order_items != null
    resource.type == "order"
    count(user.order_items) > 0
}

# JSON Type Check
allow if {
    user.form_data != null
    user.form_data.name != null
    user.form_data.age != null
    user.form_data.active != null
    resource.type == "form_data"
    is_string(user.form_data.name)
    is_number(user.form_data.age)
    is_boolean(user.form_data.active)
}

# JSON String Field
allow if {
    user.name != null
    resource.type == "text_field"
    user.name_length > 0
}

# JSON Number Field
allow if {
    user.age != null
    resource.type == "number_field"
    user.age >= 18
}

# JSON Boolean Field
allow if {
    user.verified != null
    resource.type == "boolean_field"
    user.verified == true
}

# JSON Object Field
allow if {
    user.address != null
    user.address.street != null
    user.address.city != null
    resource.type == "structured_data"
}

# JSON Merge Check
allow if {
    user.primary_data != null
    user.primary_data.name != null
    user.primary_data.email != null
    user.secondary_data != null
    user.secondary_data.phone != null
    user.secondary_data.address != null
    resource.type == "data_merge"
}
