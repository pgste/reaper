# Reaper Policy Format (.reap)

Clean, Rust-like syntax for dynamic policy loading with sub-microsecond evaluation.

## Format Design

```reap
policy document_access {
    // Policy metadata (optional)
    version: "1.0.0",
    description: "Document access control with ABAC",

    // Default decision (required)
    default: deny,

    // Rules (evaluated in order, first match wins)
    rule admin_access {
        allow if user.role == "admin"
    }

    rule department_access {
        allow if {
            user.department == resource.department &&
            resource.classification != "secret"
        }
    }

    rule clearance_check {
        deny if resource.clearance_required > user.clearance
    }
}
```

## Syntax Features

### 1. Clean, Rust-like
- Curly braces for blocks
- Familiar operators (`==`, `!=`, `>`, `<`, `>=`, `<=`)
- Boolean logic with `&&`, `||`, `!`
- Comments with `//` and `/* */`

### 2. Single-line and Multi-line Rules

```reap
// Single condition
rule simple {
    allow if user.role == "admin"
}

// Multiple conditions
rule complex {
    allow if {
        user.department == resource.department &&
        user.clearance >= resource.clearance_required &&
        !resource.archived
    }
}
```

### 3. Type Support

```reap
rule examples {
    allow if {
        user.name == "alice"              // String equality
        user.age >= 18                     // Integer comparison
        user.active == true                // Boolean
        user.score > 3.14                  // Float comparison
        user.status != null                // Null check
    }
}
```

### 4. Entity Attribute Access

```reap
user.attribute          // User entity
resource.attribute      // Resource entity
context.attribute       // Context (future)
```

## Complete Examples

### Example 1: Role-Based Access Control (RBAC)

```reap
policy rbac {
    version: "1.0.0",
    description: "Simple role-based access control",
    default: deny,

    rule admin_full_access {
        allow if user.role == "admin"
    }

    rule manager_read_access {
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

### Example 2: Attribute-Based Access Control (ABAC)

```reap
policy abac_clearance {
    version: "2.0.0",
    description: "Clearance-based document access",
    default: deny,

    // Deny suspended users first
    rule deny_suspended {
        deny if user.suspended == true
    }

    // Allow if clearance sufficient
    rule clearance_access {
        allow if {
            user.clearance >= resource.clearance_required &&
            user.department == resource.department &&
            resource.archived != true
        }
    }

    // Owner override
    rule owner_access {
        allow if {
            user.id == resource.owner_id &&
            user.status == "active"
        }
    }
}
```

### Example 3: Complex Department Access

```reap
policy department_access {
    version: "1.5.0",
    description: "Multi-factor department access control",
    default: deny,

    rule same_department_public {
        allow if {
            user.department == resource.department &&
            resource.classification == "public"
        }
    }

    rule same_department_internal {
        allow if {
            user.department == resource.department &&
            resource.classification == "internal" &&
            user.employment_type == "full-time"
        }
    }

    rule cross_department_with_approval {
        allow if {
            user.clearance >= resource.clearance_required &&
            resource.cross_department_allowed == true
        }
    }

    rule executive_override {
        allow if {
            user.role == "executive" &&
            user.clearance >= 5
        }
    }
}
```

## Loading Policies

### From File

```rust
use policy_engine::ReaperPolicy;

// Parse .reap file
let policy = ReaperPolicy::from_file("policy.reap")?;

// Load with data store
let evaluator = policy.build(store)?;

// Evaluate
let decision = evaluator.evaluate(&request)?;
```

### From String

```rust
let policy_text = r#"
    policy simple {
        default: deny,
        rule admin { allow if user.role == "admin" }
    }
"#;

let policy = ReaperPolicy::from_str(policy_text)?;
let evaluator = policy.build(store)?;
```

### From Bundle

```rust
// Load precompiled bundle
let evaluator = ReaperDSLEvaluator::from_bundle("policy.rbb", store)?;
```

## CLI Commands

### Evaluate Policy

```bash
# Evaluate .reap file
reaper eval \
    --policy policy.reap \
    --data data.json \
    --principal alice \
    --action read \
    --resource doc-1

# Output:
# Decision: allow
# Evaluation time: 234 ns
# Rule matched: admin_access
```

### Compile to Bundle

```bash
# Compile for production
reaper compile policy.reap -o policy.rbb

# Compile with optimizations
reaper compile policy.reap -o policy.rbb --optimize

# Multiple policies
reaper compile policies/*.reap -o bundle.rbb
```

### Validate Policy

```bash
# Check syntax
reaper validate policy.reap

# Validate with data
reaper validate policy.reap --data data.json
```

## Grammar Specification

```pest
// Pest grammar for .reap files

policy = { SOI ~ "policy" ~ ident ~ "{" ~ policy_body ~ "}" ~ EOI }

policy_body = {
    (metadata_field | default_field | rule)*
}

metadata_field = {
    ident ~ ":" ~ string ~ ","?
}

default_field = {
    "default" ~ ":" ~ decision ~ ","?
}

rule = {
    "rule" ~ ident ~ "{" ~ decision ~ "if" ~ condition ~ "}"
}

decision = { "allow" | "deny" }

condition = {
    condition_single |
    "{" ~ condition_expr ~ "}"
}

condition_single = { expr }

condition_expr = {
    expr ~ (("&&" | "||") ~ expr)*
}

expr = {
    "!" ~ expr |
    expr_primary
}

expr_primary = {
    entity_attr ~ op ~ value |
    entity_attr ~ op ~ entity_attr |
    "true" | "false"
}

entity_attr = { entity ~ "." ~ ident }
entity = { "user" | "resource" | "context" }

op = { "==" | "!=" | ">=" | "<=" | ">" | "<" }

value = { string | number | boolean | null }

string = { "\"" ~ (!"\"" ~ ANY)* ~ "\"" }
number = @{ "-"? ~ ASCII_DIGIT+ ~ ("." ~ ASCII_DIGIT+)? }
boolean = { "true" | "false" }
null = { "null" }
ident = @{ ASCII_ALPHA ~ (ASCII_ALPHANUMERIC | "_")* }

WHITESPACE = _{ " " | "\t" | "\n" | "\r" }
COMMENT = _{ "//" ~ (!"\n" ~ ANY)* | "/*" ~ (!"*/" ~ ANY)* ~ "*/" }
```

## Performance

| Operation | Latency | Notes |
|-----------|---------|-------|
| **Parse .reap** | 1-5 ms | One-time cost |
| **Compile to bundle** | 5-20 ms | One-time cost |
| **Load bundle** | 100-500 µs | Startup cost |
| **Evaluate (simple)** | **200 ns** | Runtime - 240x faster than Cedar |
| **Evaluate (complex)** | **500 ns** | Runtime - 100x faster than Cedar |

## Benefits Over Other Formats

| Feature | Reaper | OPA/Rego | Cedar |
|---------|--------|----------|-------|
| **Syntax** | Rust-like | Datalog-like | Custom DSL |
| **Learning curve** | Low (familiar) | High (Datalog) | Medium |
| **Type safety** | Strong | Weak | Strong |
| **Performance** | 200 ns | 100 µs | 48 µs |
| **Bundle support** | Yes | Yes (OPA bundles) | No |
| **Dynamic loading** | Yes | Yes | No |
| **Compile-time checks** | Future | No | Yes (with SDK) |

## Migration Examples

### From Cedar

```cedar
// Cedar
permit(principal, action, resource)
when {
    principal.role == "admin"
};
```

```reap
// Reaper
policy admin_policy {
    default: deny,
    rule admin { allow if user.role == "admin" }
}
```

### From OPA/Rego

```rego
# OPA
package document.access

default allow = false

allow {
    input.user.role == "admin"
}

allow {
    input.user.department == input.resource.department
}
```

```reap
// Reaper
policy document_access {
    default: deny,

    rule admin {
        allow if user.role == "admin"
    }

    rule department {
        allow if user.department == resource.department
    }
}
```
