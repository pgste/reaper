# Reaper Operations Guide

This guide covers operational procedures for running Reaper in production environments.

## Deployment Patterns

### Docker Profiles

Reaper provides Docker profiles for different deployment scenarios:

```bash
# Just the agent (standalone enforcement)
docker compose --profile engine up -d

# Agent + Platform (simple management)
docker compose --profile platform up -d

# Enterprise stack (Agent + Management + PostgreSQL)
docker compose --profile management up -d

# Full stack with observability
docker compose --profile full --profile observability up -d
```

### Profile Overview

| Profile | Services | Use Case |
|---------|----------|----------|
| `engine` | Agent | Simple policy enforcement |
| `platform` | Agent, Platform | Basic management |
| `management` | Agent, Management, PostgreSQL | Enterprise with centralized control |
| `observability` | Prometheus, Grafana, Tempo, Loki | Monitoring stack |
| `full` | All core services | Complete deployment |

## Health Checks

### Agent Health

```bash
# Health check
curl http://localhost:8080/health

# Readiness check
curl http://localhost:8080/ready

# Liveness check
curl http://localhost:8080/live
```

### Platform Health

```bash
curl http://localhost:8081/health
```

### Management Health

```bash
curl http://localhost:3000/health
```

## Metrics

### Prometheus Endpoints

All services expose Prometheus metrics at `/metrics`:

```bash
# Agent metrics
curl http://localhost:8080/metrics

# Platform metrics
curl http://localhost:8081/metrics

# Management metrics
curl http://localhost:3000/metrics
```

### Key Metrics

| Metric | Description |
|--------|-------------|
| `reaper_decisions_total` | Total policy decisions |
| `reaper_decision_duration_seconds` | Decision latency |
| `reaper_denials_total` | Total policy denials |
| `reaper_active_policies` | Number of active policies |
| `reaper_errors_total` | Total errors |
| `reaper_cache_hits_total` | Cache hit count |

## Decision Logging

### Enabling Decision Logs

Set environment variables to enable decision logging:

```bash
REAPER_DECISION_LOG_ENABLED=true
REAPER_DECISION_LOG_CAPACITY=10000
REAPER_DECISION_LOG_FILE=/var/log/reaper/decisions.ndjson
```

### Querying Decisions

```bash
# Get recent decisions
curl http://localhost:8080/api/v1/decisions

# Get decision stats
curl http://localhost:8080/api/v1/decisions/stats

# Filter by principal
curl "http://localhost:8080/api/v1/decisions?principal=alice"

# Filter by decision
curl "http://localhost:8080/api/v1/decisions?decision=deny"

# Export as NDJSON
curl -X POST http://localhost:8080/api/v1/decisions/export \
  -H "Content-Type: application/json" \
  -d '{"format": "ndjson"}'
```

### SIEM Integration

Decision logs use NDJSON format, compatible with:

- Splunk
- Elasticsearch
- Datadog
- Sumo Logic

Example log entry:

```json
{
  "timestamp": "2024-01-15T10:30:00Z",
  "decision_id": "uuid",
  "principal": "alice",
  "action": "read",
  "resource": "/api/data",
  "decision": "allow",
  "policy_id": "uuid",
  "policy_name": "data-access",
  "evaluation_time_ns": 450,
  "agent_id": "agent-1"
}
```

## Policy Management

### CLI Policy Testing

```bash
# Test a single assertion
reaper test --policy policy.reap --data entities.json \
  --principal alice --action read --resource /api \
  --expect allow

# Run a test suite
reaper test-suite --file tests.yaml
```

### Test Suite Format

```yaml
tests:
  - name: "Admin can access everything"
    policy: "policies/rbac.reap"
    data: "data/entities.json"
    principal: "admin_alice"
    action: "read"
    resource: "/admin/dashboard"
    expect: allow

  - name: "Viewer cannot write"
    policy: "policies/rbac.reap"
    data: "data/entities.json"
    principal: "viewer_bob"
    action: "write"
    resource: "/api/data"
    expect: deny
```

### Bundle Workflow

```bash
# Compile policy to bundle
reaper compile policy.reap -o policy.rbb --optimize

# View bundle info
reaper bundle info policy.rbb

# Deploy bundle
reaper bundle deploy policy.rbb --data entities.json

# Validate bundle
reaper bundle validate policy.rbb
```

## Kubernetes Deployment

### Helm Installation

```bash
# Add Reaper Helm repository
helm repo add reaper https://charts.reaper-policy.io
helm repo update

# Install with default values
helm install reaper reaper/reaper -n reaper-system --create-namespace

# Install with custom values
helm install reaper reaper/reaper -n reaper-system \
  --set agent.replicas=3 \
  --set management.enabled=true \
  --set observability.enabled=true
```

### Agent DaemonSet

Deploy agents as a DaemonSet for node-level enforcement:

```yaml
apiVersion: apps/v1
kind: DaemonSet
metadata:
  name: reaper-agent
spec:
  selector:
    matchLabels:
      app: reaper-agent
  template:
    spec:
      containers:
        - name: agent
          image: reaper-agent:latest
          ports:
            - containerPort: 8080
          env:
            - name: REAPER_DECISION_LOG_ENABLED
              value: "true"
```

### Sidecar Pattern

Deploy agent as a sidecar for application-level enforcement:

```yaml
apiVersion: v1
kind: Pod
metadata:
  name: myapp
spec:
  containers:
    - name: app
      image: myapp:latest
    - name: reaper
      image: reaper-agent:latest
      ports:
        - containerPort: 8080
```

## Troubleshooting

### Common Issues

#### Agent Not Starting

1. Check logs: `docker logs reaper-agent`
2. Verify port not in use: `lsof -i :8080`
3. Check memory limits
4. Verify config file syntax

#### Policies Not Loading

1. Check policy syntax: `reaper validate policy.reap`
2. Verify data file format
3. Check agent logs for errors
4. Ensure correct file permissions

#### High Latency

1. Check decision cache config
2. Monitor `reaper_decision_duration_seconds` metric
3. Review policy complexity
4. Consider policy optimization

#### Decision Log Issues

1. Verify `REAPER_DECISION_LOG_ENABLED=true`
2. Check file permissions for log path
3. Monitor buffer capacity
4. Check disk space

### Debug Endpoints

```bash
# Check datastore stats
curl http://localhost:8080/debug/datastore

# List loaded policies
curl http://localhost:8080/api/v1/policies
```

### Log Levels

Configure logging via `RUST_LOG`:

```bash
# Default
RUST_LOG=info

# Debug mode
RUST_LOG=debug

# Trace all
RUST_LOG=trace

# Specific component
RUST_LOG=info,reaper_agent=debug,policy_engine=trace
```

## Performance Tuning

### Decision Cache

Enable decision caching for repeated requests:

```bash
REAPER_CACHE_ENABLED=true
REAPER_CACHE_SIZE=10000
REAPER_CACHE_TTL=300
```

### Connection Pooling

Configure HTTP connection pools:

```bash
REAPER_HTTP_POOL_SIZE=100
REAPER_HTTP_TIMEOUT=30
```

### Memory Management

Monitor and tune memory:

- Decision log buffer: `REAPER_DECISION_LOG_CAPACITY`
- Policy cache: `REAPER_POLICY_CACHE_SIZE`
- Entity store: Monitor via metrics

## Backup and Recovery

### Policy Backup

```bash
# Export policies
curl http://localhost:8081/api/v1/policies > policies-backup.json

# Export bundles
curl http://localhost:3000/orgs/myorg/bundles > bundles-backup.json
```

### Database Backup (Management)

```bash
# PostgreSQL backup
pg_dump -h localhost -U reaper reaper_management > backup.sql

# Restore
psql -h localhost -U reaper reaper_management < backup.sql
```

## Security Hardening

### Network Security

1. Use TLS for all endpoints
2. Enable mTLS for agent-management communication
3. Restrict network access via firewall
4. Use private networks for internal communication

### Access Control

1. Rotate API keys regularly
2. Use short-lived JWT tokens
3. Audit all policy changes
4. Implement RBAC for management access

### Container Security

1. Run as non-root user
2. Use read-only filesystem where possible
3. Limit container capabilities
4. Scan images for vulnerabilities

## Monitoring Alerts

### Recommended Alerts

```yaml
# High denial rate
- alert: HighDenialRate
  expr: rate(reaper_denials_total[5m]) > 100
  for: 5m

# Slow evaluations
- alert: SlowEvaluations
  expr: histogram_quantile(0.99, reaper_decision_duration_seconds) > 0.001
  for: 5m

# Agent down
- alert: AgentDown
  expr: up{job="reaper-agent"} == 0
  for: 1m

# Decision log buffer full
- alert: DecisionLogBufferFull
  expr: reaper_decision_log_buffer_size / reaper_decision_log_capacity > 0.9
  for: 5m
```

## Related Documentation

- [Event-Driven Loading](../concepts/EVENT_DRIVEN_LOADING.md)
- [Bundle Format](../concepts/BUNDLE_FORMAT.md)
- [Architecture](../architecture/ARCHITECTURE.md)
