# Cedar Policy Examples for Reaper

This document contains Cedar policy examples you can use with Reaper's policy engine.

## Basic Examples

### 1. Simple Allow-All Policy
```cedar
permit(principal, action, resource);
```

### 2. Allow Specific Action
```cedar
permit(
    principal,
    action == Action::"read",
    resource
);
```

### 3. Role-Based Access Control (RBAC)
```cedar
// Allow admins to do everything
permit(
    principal,
    action,
    resource
) when {
    principal.role == "admin"
};

// Allow users to read
permit(
    principal,
    action == Action::"read",
    resource
) when {
    principal.role == "user"
};
```

### 4. Resource-Based Access
```cedar
// Allow access to specific resource type
permit(
    principal,
    action == Action::"read",
    resource
) when {
    resource.type == "document"
};

// Allow access within a folder
permit(
    principal,
    action,
    resource in Folder::"documents"
);
```

### 5. Attribute-Based Access Control (ABAC)
```cedar
// Allow if user's department matches resource owner
permit(
    principal,
    action == Action::"read",
    resource
) when {
    principal.department == resource.owner_department
};

// Allow based on time-based condition
permit(
    principal,
    action,
    resource
) when {
    context.time_of_day >= 9 && context.time_of_day < 17
};
```

### 6. Deny Policies (Explicit Forbid)
```cedar
// Explicitly deny access to sensitive resources
forbid(
    principal,
    action,
    resource
) when {
    resource.classification == "top-secret" &&
    principal.clearance_level < 5
};
```

### 7. Multi-Condition Policies
```cedar
// Complex multi-condition policy
permit(
    principal,
    action in [Action::"read", Action::"write"],
    resource
) when {
    // User must be in same department
    principal.department == resource.department &&
    // User must have required role
    (principal.role == "manager" || principal.role == "admin") &&
    // Resource must not be archived
    !resource.archived
};
```

## Advanced Examples

### 8. Hierarchical Resource Access
```cedar
// Allow access to all resources in a folder hierarchy
permit(
    principal,
    action == Action::"read",
    resource in Folder::"public/shared"
);

// Allow managers to access their team's resources
permit(
    principal,
    action,
    resource in Team::?teamId
) when {
    principal.managed_teams.contains(context.teamId)
};
```

### 9. IP-Based Access Control
```cedar
// Allow access only from trusted IP ranges
permit(
    principal,
    action,
    resource
) when {
    context.ip_address.isInRange("10.0.0.0/8") ||
    context.ip_address.isInRange("192.168.0.0/16")
};
```

### 10. Combination of Permit and Forbid
```cedar
// Allow regular access
permit(
    principal,
    action == Action::"read",
    resource
) when {
    principal.role == "user"
};

// But forbid access during maintenance window
forbid(
    principal,
    action,
    resource
) when {
    context.maintenance_mode == true
};

// Admins can access even during maintenance
permit(
    principal,
    action,
    resource
) when {
    principal.role == "admin"
};
```

## Integration with Reaper

### Creating a Cedar Policy in Reaper

```rust
use policy_engine::{EnhancedPolicy, PolicyLanguage};

let cedar_text = r#"
    permit(
        principal,
        action == Action::"read",
        resource
    ) when {
        principal.role == "viewer"
    };
"#;

let policy = EnhancedPolicy::new_with_language(
    "my-cedar-policy".to_string(),
    "RBAC for viewers".to_string(),
    PolicyLanguage::Cedar,
    cedar_text.to_string(),
)?;
```

### Evaluating a Request

```rust
use std::collections::HashMap;

let mut context = HashMap::new();
context.insert("principal".to_string(), "alice".to_string());

let request = PolicyRequest {
    resource: "document-123".to_string(),
    action: "read".to_string(),
    context,
};

let decision = engine.evaluate(&policy.id, &request)?;
println!("Decision: {:?}", decision.decision);
```

## Entity Model in Reaper

Cedar policies reference entities like `User::`, `Action::`, and `Resource::`.
In Reaper, the mapping is:

- **Principal**: Extracted from `context["principal"]` or defaults to "anonymous"
- **Action**: Taken from `request.action`
- **Resource**: Taken from `request.resource`
- **Context**: Built from all `request.context` key-value pairs

### Example Context Mapping

Request:
```rust
let mut context = HashMap::new();
context.insert("principal".to_string(), "alice".to_string());
context.insert("department".to_string(), "engineering".to_string());
context.insert("clearance".to_string(), "5".to_string());

let request = PolicyRequest {
    resource: "file-123".to_string(),
    action: "read".to_string(),
    context,
};
```

Cedar can access these as:
```cedar
permit(principal, action, resource) when {
    context.department == "engineering" &&
    context.clearance >= "3"
};
```

## Performance Characteristics

- **Cedar Evaluation Time**: Typically 1-50 milliseconds per request
- **Use Cedar When**:
  - Rich ABAC policies needed
  - Schema validation important
  - AWS compatibility desired
  - Policy expressiveness > raw speed

- **Use Simple Policies When**:
  - Sub-microsecond latency critical
  - >100K requests/second required
  - Simple resource patterns sufficient

## References

- [Cedar Policy Language Documentation](https://www.cedarpolicy.com/)
- [Cedar Rust Crate](https://docs.rs/cedar-policy/)
- [AWS Cedar on GitHub](https://github.com/cedar-policy/cedar)
