# Quick Start

Get started with Reaper in 5 minutes.

## Step 1: Start the Services

```bash
# Terminal 1: Start the agent
cargo run --bin reaper-agent

# Terminal 2: Start the platform
cargo run --bin reaper-platform
```

Wait for both services to start. You should see:

```
✓ Reaper Agent listening on http://0.0.0.0:8080
✓ Reaper Platform listening on http://0.0.0.0:8081
```

## Step 2: Write Your First Policy

Create a file `my-policy.reap`:

```reap
policy rbac_simple {
  # Admins can do anything
  permit principal.role == "admin"

  # Users can read their own resources
  permit
    principal.id == resource.owner_id
    && action == "read"
}
```

This policy allows:
- Admins to do anything
- Users to read resources they own

## Step 3: Load Test Data

Create `test-data.json`:

```json
{
  "entities": [
    {
      "type": "user",
      "id": "alice",
      "attributes": {
        "role": "admin"
      }
    },
    {
      "type": "user",
      "id": "bob",
      "attributes": {
        "role": "user"
      }
    },
    {
      "type": "resource",
      "id": "document-123",
      "attributes": {
        "owner_id": "bob",
        "type": "document"
      }
    }
  ]
}
```

## Step 4: Evaluate Policies

### Using the CLI

```bash
# Admin can do anything
reaper-cli eval \
  --policy my-policy.reap \
  --data test-data.json \
  --principal "alice" \
  --action "delete" \
  --resource "document-123"
# Output: ✅ ALLOW

# Bob can read his own document
reaper-cli eval \
  --policy my-policy.reap \
  --data test-data.json \
  --principal "bob" \
  --action "read" \
  --resource "document-123"
# Output: ✅ ALLOW

# Bob cannot delete
reaper-cli eval \
  --policy my-policy.reap \
  --data test-data.json \
  --principal "bob" \
  --action "delete" \
  --resource "document-123"
# Output: ❌ DENY
```

### Using the API

First, deploy the policy to the agent:

```bash
# Create the policy on the platform
curl -X POST http://localhost:8081/api/v1/policies \
  -H "Content-Type: application/json" \
  -d @- << 'EOF'
{
  "name": "rbac_simple",
  "format": "reap",
  "content": "policy rbac_simple {\n  permit principal.role == \"admin\"\n  permit principal.id == resource.owner_id && action == \"read\"\n}"
}
EOF

# Response: {"id": "policy-123", "name": "rbac_simple", ...}
```

Load the data into the agent:

```bash
curl -X POST http://localhost:8080/api/v1/data \
  -H "Content-Type: application/json" \
  -d @test-data.json
```

Evaluate a request:

```bash
# Check if Alice (admin) can delete
curl -X POST http://localhost:8080/api/v1/messages \
  -H "Content-Type: application/json" \
  -d '{
    "policy_id": "policy-123",
    "resource": "document-123",
    "action": "delete",
    "context": {
      "principal": "alice"
    }
  }'

# Response: {"decision": "Allow"}
```

## Step 5: Monitor Performance

Check metrics:

```bash
curl http://localhost:8080/metrics
```

Output:

```json
{
  "evaluations": {
    "total": 1523,
    "allow": 891,
    "deny": 632
  },
  "latency": {
    "mean_ns": 342,
    "p99_ns": 1250
  },
  "policies": {
    "active": 1,
    "total": 5
  }
}
```

## Common Workflows

### Hot-Swapping a Policy

Update your policy without restarting:

```bash
# Update the policy file
cat > my-policy.reap << 'EOF'
policy rbac_updated {
  # Now managers can also approve
  permit principal.role == "admin"
  permit principal.role == "manager" && action == "approve"
  permit principal.id == resource.owner_id && action == "read"
}
EOF

# Deploy the update
curl -X PUT http://localhost:8081/api/v1/policies/policy-123 \
  -H "Content-Type: application/json" \
  -d '{
    "content": "policy rbac_updated { permit principal.role == \"admin\" permit principal.role == \"manager\" && action == \"approve\" permit principal.id == resource.owner_id && action == \"read\" }"
  }'

# Verify it's live (no restart needed!)
reaper-cli eval \
  --policy my-policy.reap \
  --data test-data.json \
  --principal "manager-user" \
  --action "approve" \
  --resource "request-456"
# Output: ✅ ALLOW
```

### Testing Policies

Write BDD-style tests:

```gherkin
# features/rbac.feature
Feature: RBAC Policy

  Scenario: Admin can do anything
    Given a user "alice" with role "admin"
    And a resource "document-123"
    When alice tries to "delete" the resource
    Then the decision should be "Allow"

  Scenario: User can read own resources
    Given a user "bob" with role "user"
    And a resource "document-123" owned by "bob"
    When bob tries to "read" the resource
    Then the decision should be "Allow"

  Scenario: User cannot delete others' resources
    Given a user "bob" with role "user"
    And a resource "document-456" owned by "alice"
    When bob tries to "delete" the resource
    Then the decision should be "Deny"
```

Run the tests:

```bash
cargo test --test policy_bdd_tests
```

See [Testing Guide](../guides/testing.md) for more details.

## What's Next?

Now that you've run your first policy evaluation:

### Learn More

- **[First Policy](./first-policy.md)** - Deep dive into writing policies
- **[Policy Languages](../guides/policy-languages.md)** - Explore REAP, Cedar, YAML formats
- **[Examples](./examples.md)** - Browse example policies

### Explore Concepts

- **[Architecture](../concepts/architecture.md)** - Understand how Reaper works
- **[Policy Engine](../concepts/policy-engine.md)** - Learn about the evaluation engine
- **[Data Store](../concepts/data-store.md)** - How entity data is managed

### Deploy to Production

- **[Deployment Patterns](../deployment/deployment-patterns.md)** - Choose your deployment
- **[Sidecar Mode](../deployment/sidecar.md)** - Deploy alongside services
- **[Performance Tuning](../guides/performance-tuning.md)** - Optimize for production

## Troubleshooting

### Services Won't Start

**Problem**: Port already in use

**Solution**:
```bash
# Check what's using the port
lsof -i :8080

# Kill the process or use a different port
export REAPER_AGENT_PORT=9080
cargo run --bin reaper-agent
```

### Policy Evaluation Fails

**Problem**: "Resource entity not found"

**Solution**: Ensure you've loaded the data:
```bash
curl -X POST http://localhost:8080/api/v1/data \
  -H "Content-Type: application/json" \
  -d @test-data.json
```

### Slow Performance

**Problem**: Latency > 10µs

**Solution**: Use native REAP format instead of YAML/JSON for best performance.

See [Performance Guide](../guides/performance-tuning.md) for optimization tips.

## Getting Help

- **[Documentation](../index.md)** - Browse all docs
- **[GitHub Issues](https://github.com/your-org/reaper/issues)** - Report bugs
- **[Discussions](https://github.com/your-org/reaper/discussions)** - Ask questions
