package reaper.comprehension

import rego.v1

# Comprehension Policy — mirrors comprehension_policy.reap exactly
# Rules:
#   1. filtered_set_comprehension: numbers > 5, count >= 3
#   2. ordered_array_comprehension: items with priority "high", count >= 2
#   3. object_mapping_comprehension: active records mapping, count >= 2
#   4. multi_filter_comprehension: score > 80 && verified, count >= 2
#   5. nested_comprehension: groups with items, count >= 2
#   6. transform_comprehension: uppercase strings containing "a", count >= 2

default allow := false

# Entity lookups — .attributes shorthand
user := data.entities[input.principal.id].attributes

resource := data.entities[input.resource].attributes

# Set comprehension with filter
allow if {
    user.numbers != null
    resource.type == "set_result"
    evens := {n | some n in user.numbers; n > 5}
    count(evens) >= 3
}

# Array comprehension preserving order
allow if {
    user.items != null
    resource.type == "array_result"
    filtered := [item | some item in user.items; item.priority == "high"]
    count(filtered) >= 2
}

# Object comprehension creating mapping
allow if {
    user.records != null
    resource.type == "object_result"
    mapping := {r.id: r.value | some r in user.records; r.active == true}
    count(mapping) >= 2
}

# Multiple filters in comprehension
allow if {
    user.data != null
    resource.type == "complex_filter"
    filtered := [d | some d in user.data; d.score > 80; d.verified == true]
    count(filtered) >= 2
}

# Nested iteration comprehension
allow if {
    user.nested != null
    resource.type == "nested_result"
    groups_with_items := [g | some g in user.nested; g.items != null]
    count(groups_with_items) >= 2
}

# Comprehension with transformation
allow if {
    user.strings != null
    resource.type == "transformed_data"
    uppers := [upper(s) | some s in user.strings; contains(s, "a")]
    count(uppers) >= 2
}
