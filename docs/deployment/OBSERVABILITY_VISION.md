# Reaper Observability & Decision Streaming

**Phase 8.2: Real-Time Policy Decision Monitoring**

Complete observability for security, compliance, and debugging.

---

## 🎯 Vision: Every Decision is Observable

**Core Principle**: Every policy decision creates a rich event that flows through your observability stack in real-time.

### Decision Event Anatomy

```json
{
  "timestamp": "2025-12-14T12:45:23.123456Z",
  "decision_id": "dec_a1b2c3d4e5f6",
  "policy_id": "pol_rbac_v2",
  "policy_name": "rbac-production",
  "decision": "allow",
  "latency_ns": 1234,
  "latency_us": 1.234,

  "principal": {
    "id": "user_12345",
    "type": "User",
    "attributes": {
      "role": "engineer",
      "team": "platform",
      "clearance": 7
    }
  },

  "resource": {
    "id": "doc_secret_42",
    "type": "Document",
    "path": "/api/documents/42",
    "classification": "confidential"
  },

  "action": "read",

  "context": {
    "ip_address": "10.0.1.45",
    "user_agent": "Mozilla/5.0...",
    "request_id": "req_xyz789"
  },

  "evaluation": {
    "rules_evaluated": 3,
    "matching_rule": "same_team_access",
    "cache_hit": true
  },

  "tags": {
    "environment": "production",
    "region": "us-east-1",
    "service": "api-gateway"
  }
}
```

---

## 📊 Observability Stack

### Architecture

```
Policy Decision
    ↓
[Structured Event]
    ├→ Prometheus (Metrics)      → Grafana Dashboards
    ├→ Tracing (OpenTelemetry)   → Jaeger/Tempo
    ├→ Logging (Structured JSON) → Loki/Elasticsearch
    └→ Event Stream (Optional)   → Kafka/NATS → Analytics
```

---

## 🔢 Prometheus Metrics

### Core Metrics

**Decision Counters**:
```prometheus
# Total decisions by outcome
reaper_decisions_total{decision="allow|deny", policy="...", service="..."}

# Decisions by principal type
reaper_decisions_by_principal_total{principal_type="User|Service|Anonymous"}

# Decisions by resource type
reaper_decisions_by_resource_total{resource_type="Document|API|Database"}

# Policy denials (security events!)
reaper_denials_total{policy="...", reason="..."}
```

**Performance Metrics**:
```prometheus
# Decision latency histogram (sub-microsecond tracking)
reaper_decision_duration_seconds{policy="...", quantile="0.5|0.95|0.99"}

# Policy evaluation time
reaper_policy_evaluation_ns{policy_id="..."}

# Cache performance
reaper_cache_hits_total{cache_type="policy|regex|attribute"}
reaper_cache_misses_total{cache_type="policy|regex|attribute"}
```

**Policy Health**:
```prometheus
# Active policies
reaper_active_policies{version="..."}

# Policy load time
reaper_policy_load_duration_seconds

# Hot-swap events
reaper_policy_swaps_total{policy_id="..."}
```

**System Health**:
```prometheus
# Request throughput
reaper_requests_per_second

# Error rate
reaper_errors_total{error_type="..."}

# Concurrent evaluations
reaper_concurrent_evaluations
```

### Labels (Dimensions)

Every metric tagged with:
- `environment` (prod, staging, dev)
- `region` (us-east-1, eu-west-1, etc.)
- `service` (api-gateway, auth-service, etc.)
- `policy_id` / `policy_name`
- `decision` (allow, deny)
- `principal_type` / `resource_type`

---

## 📈 Grafana Dashboards

### 1. Executive Dashboard

**Panels**:
- Total Decisions (today, this week, this month)
- Allow vs Deny Ratio (pie chart)
- Top 10 Policies by Usage
- Decision Latency Trend (p50, p95, p99)
- Denial Rate by Service (security view)

### 2. Security Dashboard

**Focus**: Security events and access patterns

**Panels**:
- **Denials Stream**: Live feed of denied requests
- **Suspicious Activity**: Anomalies in access patterns
- **Failed Access Attempts**: By user, by resource
- **Policy Violations**: Which policies deny most
- **Privilege Escalation Attempts**: Pattern detection

**Alerts**:
- Denial rate >5% for any user
- Repeated denials from same IP (>10 in 1min)
- Access to sensitive resources denied

### 3. Performance Dashboard

**Focus**: Sub-microsecond performance monitoring

**Panels**:
- Decision Latency Distribution (heatmap)
- P99 Latency by Policy
- Cache Hit Rate (%)
- SIMD Activation Rate (for large collections)
- Slowest Policies (top 10)

**Alerts**:
- P99 latency >100µs (degradation)
- Cache hit rate <80%
- Any policy >1ms evaluation

### 4. Compliance Dashboard

**Focus**: Audit trail and compliance reporting

**Panels**:
- **Audit Log**: Full decision history with filters
- **Access Patterns**: Who accessed what, when
- **Policy Coverage**: Which resources have policies
- **Compliance Score**: % of requests with explicit policies
- **Data Access Reports**: By user, by document, by time

**Export**: CSV, JSON for compliance reports

### 5. Developer Dashboard

**Focus**: Policy development and debugging

**Panels**:
- Policy Evaluation Breakdown (which rules matched)
- Attribute Resolution Time
- Regex Cache Performance
- Bundle Load Times
- Policy Swap Success Rate

---

## 🔍 OpenTelemetry Tracing

### Trace Structure

```
Request Received
  └─ Policy Evaluation [1.2µs]
      ├─ Load Policy [0.1µs] (cache hit)
      ├─ Resolve Principal Attributes [0.3µs]
      ├─ Resolve Resource Attributes [0.3µs]
      ├─ Evaluate Rules [0.4µs]
      │   ├─ Rule 1: RBAC [0.2µs] ❌ no match
      │   └─ Rule 2: Team Access [0.2µs] ✅ ALLOW
      └─ Record Decision [0.1µs]
```

### Trace Attributes

Every span tagged with:
- `policy.id`
- `policy.name`
- `decision.outcome`
- `decision.latency_ns`
- `principal.id`
- `resource.id`
- `rule.matched`

### Distributed Tracing

Connect policy decisions to full request flow:
```
API Gateway [span: api_request]
  └─ Auth Service [span: auth_check]
      └─ Reaper Agent [span: policy_eval] ← DECISION HERE
          └─ Database [span: db_query]
```

---

## 📝 Structured Logging

### Log Levels

**INFO**: Normal decisions (sampled for high volume)
```json
{
  "level": "info",
  "msg": "Policy decision",
  "decision_id": "...",
  "decision": "allow",
  "latency_us": 1.2,
  "policy": "rbac-v2"
}
```

**WARN**: Denials (always logged)
```json
{
  "level": "warn",
  "msg": "Access denied",
  "decision_id": "...",
  "principal_id": "user_123",
  "resource": "/api/secrets/42",
  "policy": "confidential-access",
  "reason": "insufficient_clearance"
}
```

**ERROR**: System errors
```json
{
  "level": "error",
  "msg": "Policy evaluation failed",
  "error": "policy not found",
  "policy_id": "pol_missing"
}
```

### Sampling Strategy

**High Volume (>1000 RPS)**:
- ALLOW decisions: Sample 1% (configurable)
- DENY decisions: Log 100% (security critical)
- Slow decisions (>100µs): Log 100%
- Error conditions: Log 100%

---

## 🚨 Alert Rules

### Critical Alerts (PagerDuty)

```yaml
# High denial rate (potential attack)
- alert: HighDenialRate
  expr: rate(reaper_denials_total[5m]) > 100
  for: 2m
  annotations:
    summary: "High denial rate detected"
    description: "{{ $value }} denials/sec in last 5min"

# Policy evaluation failures
- alert: PolicyEvaluationFailures
  expr: rate(reaper_errors_total[5m]) > 10
  for: 1m
  annotations:
    summary: "Policy evaluation failures"

# Performance degradation
- alert: SlowPolicyEvaluation
  expr: histogram_quantile(0.99, reaper_decision_duration_seconds) > 0.0001
  for: 5m
  annotations:
    summary: "P99 latency >100µs"
```

### Warning Alerts (Slack)

```yaml
# Cache hit rate drop
- alert: LowCacheHitRate
  expr: rate(reaper_cache_hits_total[10m]) / rate(reaper_decisions_total[10m]) < 0.8
  for: 10m
  annotations:
    summary: "Cache hit rate below 80%"

# Unusual access patterns
- alert: RepeatedDenials
  expr: increase(reaper_denials_total{principal_id="..."}[1m]) > 10
  annotations:
    summary: "User {{ $labels.principal_id }} denied 10+ times in 1min"
```

---

## 🌊 Decision Event Streaming

### Event Stream (Optional - For Analytics)

**Use Cases**:
- Real-time security analytics
- Machine learning on access patterns
- Compliance data warehouse
- SIEM integration

**Destinations**:
- **Kafka**: High-throughput event stream
- **NATS**: Lightweight pub/sub
- **AWS Kinesis**: Cloud-native streaming
- **Webhook**: Custom integrations

**Event Format**:
```json
{
  "stream": "reaper.decisions",
  "version": "1.0",
  "event": { /* full decision event */ }
}
```

**Partitioning**:
- By `principal_id` (user activity)
- By `resource_type` (resource access)
- By `decision` (allow vs deny)

---

## 🎨 Real-Time Dashboards

### Live Decision Feed

**WebSocket endpoint**: `ws://agent:8080/decisions/stream`

```javascript
const ws = new WebSocket('ws://localhost:8080/decisions/stream');

ws.onmessage = (event) => {
  const decision = JSON.parse(event.data);
  console.log(`${decision.decision.toUpperCase()}: ${decision.principal.id} → ${decision.resource.path}`);

  // Update live dashboard
  updateDashboard(decision);
};
```

**Use Cases**:
- Security Operations Center (SOC) dashboard
- Real-time compliance monitoring
- Live debugging during development

---

## 🔧 Implementation Plan

### Phase 1: Prometheus Metrics (Week 1)

- [ ] Add `prometheus` crate dependency
- [ ] Create metrics module (`crates/metrics/src/prometheus.rs`)
- [ ] Instrument Agent with decision metrics
- [ ] Add `/metrics` endpoint (Prometheus format)
- [ ] Test with Prometheus scraper

### Phase 2: Grafana Dashboards (Week 1)

- [ ] Create dashboard JSON files
- [ ] Deploy Grafana with docker-compose
- [ ] Import dashboards automatically
- [ ] Configure Prometheus datasource
- [ ] Test visualization

### Phase 3: Structured Logging (Week 2)

- [ ] Add `tracing-subscriber` JSON formatter
- [ ] Emit decision events as structured logs
- [ ] Configure log levels and sampling
- [ ] Test with Loki/Elasticsearch

### Phase 4: OpenTelemetry (Week 2)

- [ ] Add `opentelemetry` crate
- [ ] Instrument policy evaluation with spans
- [ ] Export traces to Jaeger/Tempo
- [ ] Test distributed tracing

### Phase 5: Event Streaming (Week 3 - Optional)

- [ ] Add Kafka/NATS client
- [ ] Publish decision events
- [ ] Create consumer examples
- [ ] Performance testing (>10K events/sec)

---

## 📊 Example Queries

### Prometheus (PromQL)

```promql
# Decisions per second
rate(reaper_decisions_total[5m])

# Deny rate by policy
rate(reaper_denials_total[5m]) / rate(reaper_decisions_total[5m])

# P99 latency
histogram_quantile(0.99, reaper_decision_duration_seconds_bucket)

# Top 10 users by denials
topk(10, sum by (principal_id) (reaper_denials_total))

# Cache hit rate
rate(reaper_cache_hits_total[5m]) / (rate(reaper_cache_hits_total[5m]) + rate(reaper_cache_misses_total[5m]))
```

### LogQL (Loki)

```logql
# All denials in last hour
{service="reaper-agent"} | json | decision="deny" | line_format "{{.principal_id}} denied access to {{.resource}}"

# Slow evaluations
{service="reaper-agent"} | json | latency_us > 100

# Security events
{service="reaper-agent", level="warn"} | json
```

---

## 🎯 Success Metrics

### After Implementation

- ✅ **Visibility**: 100% of decisions observable
- ✅ **Performance**: <1µs overhead for metrics collection
- ✅ **Compliance**: Full audit trail with <1s latency
- ✅ **Security**: Real-time denial alerting (<5s)
- ✅ **Debugging**: Sub-microsecond trace resolution

---

## 🚀 Next Steps

1. **Immediate**: Add Prometheus metrics to Agent
2. **Day 2**: Create Grafana dashboards
3. **Day 3**: Structured logging
4. **Week 2**: OpenTelemetry tracing
5. **Week 3**: Event streaming (optional)

---

*Reaper Policy Engine - Making Every Decision Observable*
