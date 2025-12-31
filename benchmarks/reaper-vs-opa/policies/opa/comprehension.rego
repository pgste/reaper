package reaper.comprehension

import rego.v1

# Default deny
default allow := false

# Entity lookup from pre-loaded data
user := data.entities[input.principal]

# Set comprehension with filter
allow if {
    user.numbers != null
    input.resource == "set_result"
    evens := {n | n := user.numbers[_]; n > 5}
    count(evens) >= 3
}

# Array comprehension preserving order
allow if {
    user.items != null
    input.resource == "array_result"
    filtered := [item | item := user.items[_]; item.priority == "high"]
    count(filtered) >= 2
}

# Object comprehension creating mapping
allow if {
    user.records != null
    input.resource == "object_result"
    mapping := {r.id: r.value | r := user.records[_]; r.active == true}
    count(mapping) >= 2
}

# Multiple filters in comprehension
allow if {
    user.data != null
    input.resource == "complex_filter"
    filtered := [d | d := user.data[_]; d.score > 80; d.verified == true]
    count(filtered) >= 2
}

# Nested iteration comprehension
allow if {
    user.nested != null
    input.resource == "nested_result"
    groups_with_items := [g | g := user.nested[_]; g.items != null]
    count(groups_with_items) >= 2
}

# Comprehension with transformation
allow if {
    user.strings != null
    input.resource == "transformed_data"
    uppers := [upper(s) | s := user.strings[_]; contains(s, "a")]
    count(uppers) >= 2
}
