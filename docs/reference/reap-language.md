# Reaper Policy Language (.reap)

A high-performance, declarative policy language for the Reaper policy engine.

> **Every `.reap` example in this document is checked to parse under the current
> grammar** by `crates/policy-engine/tests/reference_examples_parse_tests.rs`
> (blocking in CI). If the language changes and this reference is not updated,
> the build fails — so the examples here can never drift out of sync with the
> engine.

## Design Goals

1. **Familiar syntax** - Rust-like, simpler than Rego
2. **Sub-microsecond evaluation** - Compiles to an optimized decision path
3. **Total & terminating** - Every policy decides in bounded time and stack
4. **Composable** - Build complex policies from simple, named rules
5. **Production-ready** - Bundle compilation for zero-overhead deployment

## File Format

### Basic Structure

A `.reap` file is a single `policy NAME { ... }` block containing optional
metadata fields, a required `default:` decision, and any number of named rules:

```reap
policy document_access {
    // Metadata fields (optional) — each value is a quoted string.
    name: "Document Access Control",
    version: "1.0.0",
    description: "Controls access to documents based on roles and departments",

    // Default decision (required): applied when no rule matches.
    default: deny,

    rule admin_access {
        allow if "admin" in user.roles
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

### Evaluation Semantics

Reaper is **deny-overrides**, not plain first-match:

1. **All `deny` rules are evaluated first.** If any matches, the decision is
   `deny` and evaluation stops — no `allow` can override an explicit `deny`.
2. **Then `allow` rules are evaluated,** in order. The first matching `allow`
   decides.
3. **If no rule matches,** the policy's `default:` decision applies.

Write allows as the common case and reserve `deny` rules for hard overrides
(suspension, consent revocation, kill-switches) that must win regardless of
order.

## Syntax Reference

### Policy Declaration

Every file is exactly one policy block, named with an identifier:

```reap
policy my_policy_name {
    default: deny,
}
```

### Metadata Fields (Optional)

Metadata is expressed as `key: "value",` fields inside the policy body. Values
are always quoted strings:

```reap
policy documented {
    name: "Human-readable name",
    version: "1.0.0",
    description: "Policy description",
    author: "Team name",
    default: deny,
}
```

`language_version` is a reserved metadata field that pins the `.reap` language
version a policy targets; see
[DSL_COMPATIBILITY.md](./DSL_COMPATIBILITY.md).

### Default Decision (Required)

```reap
policy defaulted {
    default: allow,
}
```

`default: deny,` is the safe posture and what production policies should use.

### Rules

Each rule names a decision (`allow` or `deny`) and a condition. The condition
follows `if`, either as a single expression or wrapped in braces:

```reap
policy rule_forms {
    default: deny,

    // Single-expression condition.
    rule short_form {
        allow if user.role == "admin"
    }

    // Braced condition (idiomatic for multi-term conditions).
    rule block_form {
        allow if {
            user.role == "admin" &&
            user.status == "active"
        }
    }
}
```

### Conditions

Conditions are boolean expressions over the request entities. Terms combine
with `&&` (and), `||` (or), `!` (not), and parentheses. There is no implicit
AND — join terms explicitly with `&&`.

#### Equality

```reap
user.role == "admin"
```

Compare an attribute against another attribute (cross-entity):

```reap
user.department == resource.department
```

#### Comparison Operators

Ordered comparisons on numbers (`>`, `>=`, `<`, `<=`):

```reap
user.clearance >= resource.clearance_required
```

Inequality (`!=`):

```reap
resource.classification != "secret"
```

#### Membership (`in`)

Test whether a value is a member of a list-valued attribute:

```reap
"admin" in user.roles
```

#### Boolean Logic

AND — every term must hold:

```reap
user.role == "admin" && user.status == "active"
```

OR — any term may hold:

```reap
user.role == "admin" || user.role == "super_admin"
```

NOT — prefix negation of a parenthesised expression:

```reap
!(user.role == "guest")
```

Negation applies to a boolean *expression*, not a bare attribute: to test a
boolean attribute, compare it explicitly — `user.suspended == false` rather than
`!user.suspended`.

Grouping with parentheses:

```reap
(user.role == "admin" || user.role == "moderator") && resource.archived != true
```

#### Always True

A rule that always matches uses the boolean literal `true`:

```reap
policy allow_all {
    default: deny,
    rule everything {
        allow if true
    }
}
```

## Totality & Limits

The Reaper DSL is **total and terminating by construction**. Every well-formed
policy evaluates to a decision in bounded time and bounded stack; there is no
input — however adversarial — that can make the parser, compiler, or evaluator
recurse without limit.

- **Maximum nesting depth = 64** (default). This bounds the *syntactic* nesting
  of a condition or expression: parenthesised groups `(...)`, prefix negations
  `!`, method-call chains, and function-argument nesting. Chained `&&`/`||` are
  flat (not nested) and do not count toward the depth, so ordinary policies with
  many `AND`/`OR` terms are unaffected.
- A policy whose nesting exceeds the limit is **rejected with an
  `InvalidPolicy` error at parse or compile time** — before it is ever
  deployed or evaluated. It never fails at request time.
- The limit is enforced identically for every policy format: `.reap` source,
  and the YAML/JSON representations (which construct the same AST).
- The cap is configurable via the `REAPER_MAX_NESTING_DEPTH` environment
  variable for the rare deployment that needs deeper (or shallower) policies;
  raising or lowering it requires no code change. A value of `0` or an
  unparseable value falls back to the default of 64.

This is a deliberate design choice (Plan 05, ADR-2): an authorization language
should be a total function of its input, not a general-purpose language that can
diverge. A hard, documented cap is simpler and safer than growing the stack to
accommodate pathologically deep policies.

## Data Types

### Supported Types

- **String**: `"value"`
- **Integer**: `42`, `-10`, `1000`
- **Float**: `3.14`, `-0.5`
- **Boolean**: `true`, `false`
- **Null**: `null`

### Type Strictness

Reaper is strongly typed. A comparison between incompatible types evaluates to
`false` rather than coercing — so compare like against like:

```reap
user.age == 18
```

Writing `user.age == "18"` (integer attribute against a string literal) is
`false`, because the integer `18` and the string `"18"` are different types.
This strictness is a frozen part of the language contract
([DSL_COMPATIBILITY.md](./DSL_COMPATIBILITY.md)).

## Entity References

Conditions read attributes from the request entities, addressed as
`entity.attribute`:

- `user.<attr>` — the principal's attributes (roles, department, clearance…).
- `resource.<attr>` — the target resource's attributes.
- `context.<attr>` — request context. Notably `context.action` (the action
  being attempted) and `context.principal` (the principal identifier).
- `actor.<attr>` — the acting identity, when it differs from `user`
  (delegation / on-behalf-of).

A complete policy putting these together:

```reap
policy entity_reference_demo {
    default: deny,

    rule owner_reads_own {
        allow if {
            context.action == "read" &&
            context.principal == resource.owner
        }
    }

    rule same_department_active {
        allow if {
            user.department == resource.department &&
            user.status == "active"
        }
    }
}
```

## Advanced Features

### Multiple Rules & Explicit Denies

Because deny rules are evaluated before allow rules, an explicit `deny` acts as
a hard override no matter where it appears textually:

```reap
policy access_control {
    default: deny,

    // Hard override: suspended users are denied regardless of any allow.
    rule deny_suspended {
        deny if user.status == "suspended"
    }

    rule admin_full_access {
        allow if "admin" in user.roles
    }

    rule department_access {
        allow if {
            user.department == resource.department &&
            resource.classification != "secret"
        }
    }

    rule owner_access {
        allow if user.id == resource.owner_id
    }
}
```

### Complex ABAC Policies

```reap
policy document_abac {
    name: "Enterprise Document Access",
    version: "2.0.0",
    default: deny,

    // Deny suspended users immediately (deny-overrides).
    rule deny_suspended_users {
        deny if user.suspended == true
    }

    // Allow if clearance sufficient and same department.
    rule clearance_check {
        allow if {
            user.clearance >= resource.clearance_required &&
            user.department == resource.department &&
            resource.archived != true
        }
    }

    // Owners are always allowed while active.
    rule owner_override {
        allow if {
            user.id == resource.owner_id &&
            user.status == "active"
        }
    }
}
```

### Violation Messages (Check Mode)

A `deny` rule may carry a human-readable `with message` clause, surfaced when
the rule fires in check/lint mode (e.g. scanning configuration for violations):

```reap
policy bucket_guard {
    default: allow,
    rule no_public_buckets {
        deny with message "bucket must not be public" if resource.public == true
    }
}
```

## Bundle Format (.rbb)

Reaper compiles `.reap` files into binary bundles for maximum performance.

### Creating a Bundle

```bash
# Compile a single policy
reaper compile policy.reap -o policy.rbb

# Compile multiple policies
reaper compile policies/*.reap -o bundle.rbb
```

### Bundle Structure

```
.rbb (Reaper Binary Bundle)
├── Metadata
│   ├── Language version
│   ├── Compilation timestamp
│   └── Source checksums
├── Interned Strings
│   └── All strings pre-interned for zero-cost lookups
├── Compiled Rules
│   └── Optimized condition trees
└── Index
    └── Fast rule lookup
```

A bundle records the `.reap` language version it was compiled against; an older
engine **fails closed** on a newer-versioned bundle rather than
misinterpreting it (see [DSL_COMPATIBILITY.md](./DSL_COMPATIBILITY.md)).

## CLI Usage

### Evaluate a Policy

```bash
reaper eval \
    --policy policy.reap \
    --data data.json \
    --principal alice \
    --action read \
    --resource document-1
```

### Compile to a Bundle

```bash
reaper compile policy.reap -o policy.rbb
```

### Validate a Policy

```bash
reaper validate policy.reap
reaper validate policy.reap --data data.json
```

### Test a Policy

```bash
reaper test policy.reap \
    --data data.json \
    --principal alice --action read --resource document-1 \
    --expect allow
```

## Best Practices

### 1. Default deny, allow explicitly

```reap
policy least_privilege {
    default: deny,
    rule allow_access {
        allow if user.clearance >= resource.clearance_required
    }
}
```

### 2. Use explicit denies only for hard overrides

Deny rules win over every allow, so reserve them for conditions that must never
be overridden — suspension, revoked consent, fraud kill-switches:

```reap
policy fraud_guard {
    default: deny,
    rule block_high_fraud_score {
        deny if context.fraud_score > 0.9
    }
    rule normal_access {
        allow if user.clearance >= resource.clearance_required
    }
}
```

### 3. Keep conditions simple

Prefer several small, clearly-named rules over one deeply-nested condition:

```reap
policy readable {
    default: deny,
    rule department_access {
        allow if {
            user.department == resource.department &&
            user.status == "active" &&
            resource.archived != true
        }
    }
}
```

### 4. Comment the business intent

```reap
policy commented {
    default: deny,
    // Business rule: all employees can read public resources.
    rule employee_public_access {
        allow if {
            user.type == "employee" &&
            resource.classification == "public"
        }
    }
}
```

## Examples

### Example 1: Role-Based Access (RBAC)

```reap
policy rbac_simple {
    default: deny,

    rule admin_access {
        allow if "admin" in user.roles
    }

    rule user_own_resources {
        allow if user.id == resource.owner_id
    }
}
```

### Example 2: Department-Based ABAC

```reap
policy department_abac {
    name: "Department Access Control",
    version: "1.0.0",
    default: deny,

    rule same_department {
        allow if {
            user.department == resource.department &&
            user.status == "active" &&
            resource.archived != true
        }
    }

    rule cross_department_with_clearance {
        allow if {
            user.clearance >= resource.clearance_required &&
            user.status == "active"
        }
    }
}
```

### Example 3: Clearance-Based Security

```reap
policy clearance_control {
    default: deny,

    rule sufficient_clearance {
        allow if {
            user.clearance >= resource.clearance_required &&
            user.background_check == true &&
            user.access_revoked != true
        }
    }

    rule owner_override {
        allow if {
            user.id == resource.owner_id &&
            user.clearance >= 1
        }
    }
}
```

## Migration from Other Engines

### From Rego (OPA)

```rego
# OPA Rego
package example

default allow = false

allow {
    input.user.role == "admin"
}
```

Becomes:

```reap
policy example {
    default: deny,
    rule admin_access {
        allow if user.role == "admin"
    }
}
```

### From Cedar

```cedar
permit(principal, action, resource)
when {
    principal.role == "admin"
};
```

Becomes:

```reap
policy example_from_cedar {
    default: deny,
    rule admin_access {
        allow if user.role == "admin"
    }
}
```

## Future Enhancements

1. **Imports** - Reuse rules across policies
2. **Schemas** - Type validation for entities
3. **Additional builtins** - Growing the standard library (regex, time, JWT and
   ReBAC traversal builtins already ship; see the policy library for usage)

Any change that alters an existing policy's decision is a breaking change gated
behind a language-version bump and the frozen decision corpus — see
[DSL_COMPATIBILITY.md](./DSL_COMPATIBILITY.md).
