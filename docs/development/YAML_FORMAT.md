# Reaper YAML/JSON Policy Format

## Overview

Reaper policies can be defined in three formats:
1. **`.reap`** - Rust-like DSL (native format, most concise)
2. **`.yaml`** - YAML format (human-friendly, structured)
3. **`.json`** - JSON format (machine-friendly, programmatic)

All three formats compile to the same internal representation and have identical runtime performance.

## Schema

### Top-Level Policy Structure

```yaml
name: string                    # Policy name
version: string                 # Semantic version
description: string (optional)  # Human-readable description
default_decision: allow | deny  # Default action if no rules match

rules:                          # List of authorization rules
  - name: string
    decision: allow | deny
    condition: <Condition>
```

### Condition Types

#### 1. Simple Comparison

```yaml
condition:
  operator: equal | not_equal
  left:
    entity: user | resource | context
    attribute: string
  right:
    value: string | number | boolean
```

**Example:**
```yaml
# user.role == "admin"
condition:
  operator: equal
  left:
    entity: user
    attribute: role
  right:
    value: "admin"
```

#### 2. Logical Operators

```yaml
condition:
  operator: and | or
  conditions:
    - <Condition>
    - <Condition>
    # ... more conditions
```

**Example:**
```yaml
# user.role == "manager" && resource.type == "report"
condition:
  operator: and
  conditions:
    - operator: equal
      left: {entity: user, attribute: role}
      right: {value: "manager"}
    - operator: equal
      left: {entity: resource, attribute: type}
      right: {value: "report"}
```

#### 3. Attribute-to-Attribute Comparison

```yaml
condition:
  operator: equal | not_equal
  left:
    entity: user | resource
    attribute: string
  right:
    entity: user | resource
    attribute: string
```

**Example:**
```yaml
# user.id == resource.owner_id
condition:
  operator: equal
  left:
    entity: user
    attribute: id
  right:
    entity: resource
    attribute: owner_id
```

## Complete Examples

### RBAC Policy (YAML)

```yaml
name: rbac_simple
version: "1.0.0"
description: "Simple role-based access control with ownership"
default_decision: deny

rules:
  - name: admin_full_access
    description: "Admins can do anything"
    decision: allow
    condition:
      operator: equal
      left:
        entity: user
        attribute: role
      right:
        value: "admin"

  - name: manager_reports
    description: "Managers can read reports"
    decision: allow
    condition:
      operator: and
      conditions:
        - operator: equal
          left:
            entity: user
            attribute: role
          right:
            value: "manager"
        - operator: equal
          left:
            entity: resource
            attribute: type
          right:
            value: "report"

  - name: user_own_resources
    description: "Users can access their own resources"
    decision: allow
    condition:
      operator: equal
      left:
        entity: user
        attribute: id
      right:
        entity: resource
        attribute: owner_id
```

### RBAC Policy (JSON)

```json
{
  "name": "rbac_simple",
  "version": "1.0.0",
  "description": "Simple role-based access control with ownership",
  "default_decision": "deny",
  "rules": [
    {
      "name": "admin_full_access",
      "description": "Admins can do anything",
      "decision": "allow",
      "condition": {
        "operator": "equal",
        "left": {
          "entity": "user",
          "attribute": "role"
        },
        "right": {
          "value": "admin"
        }
      }
    },
    {
      "name": "manager_reports",
      "description": "Managers can read reports",
      "decision": "allow",
      "condition": {
        "operator": "and",
        "conditions": [
          {
            "operator": "equal",
            "left": {"entity": "user", "attribute": "role"},
            "right": {"value": "manager"}
          },
          {
            "operator": "equal",
            "left": {"entity": "resource", "attribute": "type"},
            "right": {"value": "report"}
          }
        ]
      }
    },
    {
      "name": "user_own_resources",
      "description": "Users can access their own resources",
      "decision": "allow",
      "condition": {
        "operator": "equal",
        "left": {"entity": "user", "attribute": "id"},
        "right": {"entity": "resource", "attribute": "owner_id"}
      }
    }
  ]
}
```

### Equivalent .reap Format

```rust
policy rbac_simple {
    version: "1.0.0",
    description: "Simple role-based access control with ownership",
    default: deny,

    rule admin_full_access {
        allow if user.role == "admin"
    }

    rule manager_reports {
        allow if {
            user.role == "manager" &&
            resource.type == "report"
        }
    }

    rule user_own_resources {
        allow if user.id == resource.owner_id
    }
}
```

## ABAC Example (YAML)

```yaml
name: abac_clearance
version: "2.0.0"
description: "Attribute-based access with clearances"
default_decision: deny

rules:
  - name: deny_suspended_users
    description: "Block suspended users immediately"
    decision: deny
    condition:
      operator: equal
      left: {entity: user, attribute: suspended}
      right: {value: true}

  - name: clearance_and_department
    description: "Allow with clearance match and department"
    decision: allow
    condition:
      operator: and
      conditions:
        - operator: equal
          left: {entity: user, attribute: clearance_match}
          right: {value: true}
        - operator: equal
          left: {entity: user, attribute: department}
          right: {entity: resource, attribute: department}
        - operator: not_equal
          left: {entity: resource, attribute: archived}
          right: {value: true}
```

## ReBAC Example (YAML)

```yaml
name: rebac_relationships
version: "1.0.0"
description: "Relationship-based access control"
default_decision: deny

rules:
  - name: owner_full_access
    description: "Owners have full access"
    decision: allow
    condition:
      operator: equal
      left: {entity: user, attribute: id}
      right: {entity: resource, attribute: owner_id}

  - name: team_member_access
    description: "Team members can access team resources"
    decision: allow
    condition:
      operator: and
      conditions:
        - operator: equal
          left: {entity: user, attribute: team_id}
          right: {entity: resource, attribute: team_id}
        - operator: not_equal
          left: {entity: user, attribute: team_role}
          right: {value: "pending"}

  - name: shared_access
    description: "Shared resources"
    decision: allow
    condition:
      operator: equal
      left: {entity: user, attribute: id}
      right: {entity: resource, attribute: shared_with_user}
```

## Comparison: .reap vs YAML vs JSON

| Aspect | .reap | YAML | JSON |
|--------|-------|------|------|
| **Conciseness** | Most concise | Moderate | Most verbose |
| **Readability** | High (Rust-like) | High (indentation) | Moderate (brackets) |
| **Editor Support** | Custom | Excellent | Excellent |
| **Schema Validation** | Parse-time | IDE/tools | IDE/tools |
| **Programmatic Generation** | Hard | Easy | Easiest |
| **Human Authoring** | Best | Good | OK |
| **Performance** | Same (compiled to AST) | Same | Same |

## Usage

### CLI

```bash
# Evaluate with YAML policy
reaper eval --policy policy.yaml --data data.json \
    --principal user_1 --action read --resource doc_1

# Evaluate with JSON policy
reaper eval --policy policy.json --data data.json \
    --principal user_1 --action read --resource doc_1

# Compile YAML to bundle
reaper compile --input policy.yaml --output policy.rbb

# Validate YAML policy
reaper validate --policy policy.yaml
```

### Programmatic (Rust)

```rust
use policy_engine::ReaperPolicy;

// Load from YAML
let policy = ReaperPolicy::from_yaml_file("policy.yaml")?;

// Load from JSON
let policy = ReaperPolicy::from_json_file("policy.json")?;

// Auto-detect format
let policy = ReaperPolicy::from_file("policy.yaml")?;  // detects .yaml
let policy = ReaperPolicy::from_file("policy.json")?;  // detects .json
let policy = ReaperPolicy::from_file("policy.reap")?;  // detects .reap
```

## Best Practices

### When to Use Each Format

**Use .reap when:**
- Authoring policies by hand
- You want concise, readable policies
- Your team knows Rust-like syntax

**Use YAML when:**
- Need IDE support with schema validation
- Generating policies from templates
- Team prefers YAML over custom DSL
- Integration with YAML-based tooling

**Use JSON when:**
- Programmatically generating policies
- REST API integration
- Need strict schema validation
- Machine-to-machine communication

### Tips

1. **Start Simple**: Begin with YAML for readability
2. **Use Descriptions**: Add `description` fields for documentation
3. **Validate Often**: Use `reaper validate` during development
4. **Convert for Deployment**: Compile to `.rbb` bundles for production
5. **Version Control**: Use YAML/JSON in git (easier diffs than bundles)

## Schema Validation

### JSON Schema

A JSON Schema is available for validation in editors:

```bash
# Validate against schema
jsonschema -i policy.json schema/policy.schema.json
```

### YAML Schema

YAML files can be validated using the same JSON Schema (YAML is JSON-compatible).

## Error Messages

The parser provides clear error messages with context:

```yaml
# Invalid operator
condition:
  operator: greater_than  # ❌ Not supported for string comparisons
  left: {entity: user, attribute: role}
  right: {value: "admin"}
```

**Error:**
```
Error: Invalid operator 'greater_than' for literal value comparisons
  --> policy.yaml:3:13
  |
3 |   operator: greater_than
  |             ^^^^^^^^^^^^
  |
  = note: Use 'equal' or 'not_equal' for string/boolean comparisons
```

## Migration from .reap

### Automated Conversion (Future)

```bash
# Convert .reap to YAML
reaper convert --from policy.reap --to policy.yaml --format yaml

# Convert .reap to JSON
reaper convert --from policy.reap --to policy.json --format json
```

### Manual Conversion

See examples above for equivalent policies in all three formats.

## Limitations

### Current Limitations

1. **No Greater/Less Than**: Currently only `equal` and `not_equal` operators
   - Rationale: String comparisons don't have meaningful ordering
   - Future: May add numeric comparison operators

2. **No Functions**: No built-in functions like `contains()`, `matches()`
   - Rationale: Keep policies simple and auditable
   - Future: May add approved functions

3. **No Variables**: Can't define reusable sub-expressions
   - Workaround: Use multiple rules with similar conditions

These limitations apply to ALL formats (.reap, YAML, JSON) as they share the same AST.

## Performance

All formats compile to the same internal representation:
- **Parse time**: YAML ~1-5ms, JSON ~0.5-2ms, .reap ~1-5ms
- **Evaluation time**: Identical (560-1,665ns)
- **Memory**: Identical after compilation

**Recommendation**: Choose based on authoring preference, not performance.
