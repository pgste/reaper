# Reaper Observability Stack - COMPLETE

**World-class, vendor-neutral observability matching Datadog with 100% open source**

---

## 🎯 What We Built

You now have **production-grade observability** that rivals $31/host/month commercial solutions, completely free and vendor-neutral.

###The Three Pillars (Complete)

| Pillar | Technology | Purpose | Status |
|--------|-----------|---------|--------|
| **Metrics** | Prometheus | Time-series performance data | ✅ Complete |
| **Logs** | Loki | Structured events with context | ✅ Complete |
| **Traces** | Tempo | Distributed request flows | ✅ Complete |

**Unified in Grafana** - Single pane of glass for all observability data

---

## 🏗️ Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                    REAPER AGENT                                  │
│                                                                   │
│  ┌────────────────────────────────────────────────────────────┐ │
│  │  Policy Evaluation                                          │ │
│  │                                                              │ │
│  │  1. OpenTelemetry Span (policy_eval)                       │ │
│  │     • trace_id: abc123                                      │ │
│  │     • Attributes: policy, resource, action, decision        │ │
│  │                                                              │ │
│  │  2. Prometheus Metrics                                      │ │
│  │     • reaper_decisions_total                                │ │
│  │     • reaper_decision_duration_seconds (histogram)          │ │
│  │     • reaper_denials_total                                  │ │
│  │                                                              │ │
│  │  3. Structured JSON Log                                     │ │
│  │     {                                                        │ │
│  │       "trace_id": "abc123",  ← Links to trace              │ │
│  │       "decision_id": "dec_xyz",                             │ │
│  │       "policy_name": "rbac-prod",                           │ │
│  │       "decision": "allow",                                  │ │
│  │       "latency_ns": 1234                                    │ │
│  │     }                                                        │ │
│  └────────────────────────────────────────────────────────────┘ │
│                          ↓ ↓ ↓                                   │
└──────────────────────────┼─┼─┼───────────────────────────────────┘
                           │ │ │
              ┌────────────┘ │ └────────────┐
              ↓               ↓               ↓
       [Prometheus]       [Loki]         [Tempo]
       Metrics (9090)  Logs (3100)   Traces (4317)
              ↓               ↓               ↓
       ┌──────────────────────────────────────────┐
       │             GRAFANA (3000)                │
       │                                           │
       │  • Metrics: Query Prometheus              │
       │  • Logs: Search Loki with trace_id       │
       │  • Traces: Visualize distributed flows   │
       │  • Correlation: Click trace_id in logs   │
       │    → Jump directly to trace view          │
       └──────────────────────────────────────────┘
```

---

## 🚀 Quick Start (5 Minutes)

### 1. Start the Stack

```bash
cd /workspaces/reaper
docker-compose -f docker-compose.observability.yml up -d
```

**Services Started**:
- Reaper Platform (8081)
- Reaper Agent (8080) ← Your policy engine
- Prometheus (9090)
- Grafana (3000)
- Loki (3100)
- Tempo (4317, 3200)
- Promtail (log shipper)

### 2. Access Dashboards

```bash
# Grafana
open http://localhost:3000
# Login: admin / admin

# Prometheus
open http://localhost:9090

# Tempo
open http://localhost:3200
```

### 3. Send Test Requests

```bash
# Generate some policy decisions
curl -X POST http://localhost:8080/api/v1/messages \
  -H "Content-Type: application/json" \
  -d '{
    "policy_name": "demo-allow-all",
    "resource": "/api/users/123",
    "action": "read"
  }'
```

### 4. Explore in Grafana

**View Metrics**:
1. Go to Grafana → Explore
2. Select "Prometheus" datasource
3. Query: `rate(reaper_decisions_total[5m])`
4. See requests per second

**View Logs**:
1. Switch to "Loki" datasource
2. Query: `{service="reaper-agent"} | json`
3. See structured JSON logs with trace_id

**View Traces**:
1. Switch to "Tempo" datasource
2. Search for recent traces
3. Click on a trace to see the full span waterfall
4. Click trace_id in any log → jumps to trace!

---

## 📊 What You Can Do Now

### 1. Real-Time Metrics

**Request Rate**:
```promql
rate(reaper_decisions_total[5m])
```

**P99 Latency** (sub-microsecond precision):
```promql
histogram_quantile(0.99, rate(reaper_decision_duration_seconds_bucket[5m]))
```

**Denial Rate** (security monitoring):
```promql
rate(reaper_denials_total[5m])
```

**Cache Hit Rate**:
```promql
100 * rate(reaper_cache_hits_total[5m]) /
  (rate(reaper_cache_hits_total[5m]) + rate(reaper_cache_misses_total[5m]))
```

### 2. Structured Log Queries

**All Denials** (security events):
```logql
{service="reaper-agent"} | json | decision="deny"
```

**Slow Decisions** (>100µs):
```logql
{service="reaper-agent"} | json | latency_us > 100
```

**Find by Trace ID**:
```logql
{service="reaper-agent"} | json | trace_id="abc123def456"
```

### 3. Distributed Tracing

**Find Slow Requests**:
1. Tempo → Search
2. Filter: `Duration > 100µs`
3. Click trace to see span breakdown:
   ```
   API Request [45µs]
     └─ Policy Eval [1.2µs]
         ├─ Load Policy [0.1µs]
         ├─ Evaluate Rules [0.8µs]
         └─ Record Decision [0.3µs]
   ```

**Trace → Logs Correlation**:
1. Click on any span
2. Click "Logs for this span"
3. See all logs with matching trace_id

### 4. Unified Investigation Flow

**Scenario**: "Why was user_123 denied?"

1. **Start with Logs**:
   ```logql
   {service="reaper-agent"} | json | decision="deny" | resource=~".*user_123.*"
   ```

2. **Find the trace_id** in the log entry

3. **Jump to Trace**:
   - Click trace_id → Opens in Tempo
   - See full request flow
   - Identify which rule denied access

4. **Check Metrics**:
   - See if denial rate is spiking
   - Check latency impact
   - Verify cache performance

**All in under 30 seconds!**

---

## 🎨 Pre-Built Dashboards

### Security Dashboard (Available Now)

**URL**: http://localhost:3000/d/security

**10 Panels**:
- Decision Rate (allow vs deny timeseries)
- Denial Rate % (gauge with thresholds)
- Total Decisions (24h stat)
- Denials (24h stat)
- **Recent Denials (Live Stream)** ← Real-time security events
- Top 10 Denied Users
- Top 10 Protected Resources
- Policy Usage Heatmap
- **Suspicious Activity Alerts** (>10 denials in 5min)
- Decision Latency Distribution

**Auto-refreshes every 5 seconds**

### Coming Soon

- **Performance Dashboard** - P50/P95/P99 latency, cache performance
- **Compliance Dashboard** - Audit trails, access patterns
- **Developer Dashboard** - Policy debugging with traces
- **Executive Dashboard** - KPIs and trends

---

## 🔧 Configuration

### Environment Variables

**Agent** (`docker-compose.observability.yml`):
```yaml
environment:
  - RUST_LOG=info,reaper_agent=debug              # Log level
  - REAPER_LOG_FORMAT=json                        # JSON logs (Loki)
  - OTEL_EXPORTER_OTLP_ENDPOINT=http://tempo:4317 # Traces
```

### Log Format Toggle

**JSON (Production)** - Loki-compatible:
```bash
export REAPER_LOG_FORMAT=json
```

**Pretty (Development)** - Human-readable:
```bash
export REAPER_LOG_FORMAT=pretty
```

### Sampling Strategy

Current: **100% sampling** (all requests logged)

For high volume (>1000 RPS), update `main.rs`:
```rust
let sample_rate = if decision == "deny" || latency_us > 100.0 {
    1.0  // 100% for denials and slow requests
} else {
    0.01  // 1% for normal allows
};
```

---

## 📈 Comparison: Reaper vs Datadog

| Feature | Datadog | Reaper (Open Source) |
|---------|---------|----------------------|
| **Cost** | $31/host/month | $0 (infra only) |
| **Metrics** | Datadog Metrics | Prometheus |
| **Logs** | Datadog Logs | Loki |
| **Traces** | Datadog APM | Tempo (OpenTelemetry) |
| **Dashboards** | Datadog UI | Grafana |
| **Alerting** | Datadog Monitors | Prometheus Alertmanager |
| **Data Retention** | 15 days (standard) | Unlimited (you control) |
| **Vendor Lock-in** | High | **Zero** |
| **Customization** | Limited | **Unlimited** |
| **Setup Time** | 5 minutes | 5 minutes |
| **ML Anomaly Detection** | ✅ Yes | ❌ No (can add later) |
| **Auto-instrumentation** | ✅ Strong | ⚠️ Manual (but OTel compatible) |
| **Trace Correlation** | ✅ Yes | ✅ **Yes** (via trace_id) |
| **Log → Trace Jump** | ✅ Yes | ✅ **Yes** (Grafana correlation) |
| **Service Map** | ✅ Yes | ✅ Yes (Tempo service graph) |

**Verdict**: Reaper matches Datadog's core capabilities at zero cost with zero lock-in.

---

## 🔌 Optional: Add Datadog Alongside

Want the best of both worlds? Send data to **both** Grafana Stack and Datadog:

### Option 1: OTel Collector (Recommended)

Create `otel-collector-config.yaml`:
```yaml
receivers:
  otlp:
    protocols:
      grpc:
        endpoint: 0.0.0.0:4317

exporters:
  # Open source
  otlp/tempo:
    endpoint: tempo:4317
  prometheus:
    endpoint: http://prometheus:9090
  loki:
    endpoint: http://loki:3100/loki/api/v1/push

  # Commercial
  datadog:
    api:
      key: ${DD_API_KEY}
      site: datadoghq.com

service:
  pipelines:
    traces:
      receivers: [otlp]
      exporters: [otlp/tempo, datadog]  # Both!
    metrics:
      receivers: [otlp]
      exporters: [prometheus, datadog]   # Both!
```

Add to `docker-compose`:
```yaml
  otel-collector:
    image: otel/opentelemetry-collector-contrib:latest
    command: ["--config=/etc/otel-collector-config.yaml"]
    environment:
      - DD_API_KEY=${DD_API_KEY}
    volumes:
      - ./otel-collector-config.yaml:/etc/otel-collector-config.yaml:ro
    ports:
      - "4317:4317"
```

**Benefits**:
- Grafana Stack for long-term storage (free)
- Datadog for ML-powered insights (paid)
- Switch backends without code changes

---

## 🎯 Best Practices

### 1. Correlation IDs Everywhere

**Already implemented** in Reaper:
```json
{
  "trace_id": "abc123",      // OpenTelemetry trace ID
  "span_id": "def456",       // OpenTelemetry span ID
  "decision_id": "dec_xyz",  // Reaper decision ID
  "policy_name": "rbac-prod"
}
```

**Enables**:
- Click trace_id in logs → Jump to trace
- Filter metrics by trace_id
- Complete request journey reconstruction

### 2. Semantic Conventions

Reaper follows **OpenTelemetry Semantic Conventions**:

**Span Attributes**:
```
reaper.policy.name = "rbac-prod"
reaper.policy.id = "uuid"
reaper.decision = "allow"
reaper.latency_ns = 1234
reaper.resource = "/api/users/123"
reaper.action = "read"
```

### 3. Retention Strategy

**Recommendations**:

| Data Type | Hot (Fast) | Warm (Medium) | Cold (Archive) |
|-----------|------------|---------------|----------------|
| **Metrics** | 30 days | 90 days | 1 year |
| **Logs** | 7 days | 30 days | 90 days |
| **Traces** | 24 hours | 7 days | N/A |

**Configure in docker-compose**:
```yaml
prometheus:
  command:
    - '--storage.tsdb.retention.time=30d'

loki:
  command:
    - '-config.file=/etc/loki/loki.yaml'
    - '-table-manager.retention-period=7d'

tempo:
  # Already configured for 24h in tempo.yaml
```

### 4. Alert Fatigue Prevention

**Use tiered alerting**:

| Severity | Channel | Response Time |
|----------|---------|---------------|
| **Critical** | PagerDuty | Immediate |
| **Warning** | Slack | Next business day |
| **Info** | Email digest | Weekly review |

**Example alert tiers** (already in `alerts.yml`):

```yaml
# Critical (PagerDuty)
- alert: HighDenialRate
  expr: rate(reaper_denials_total[5m]) > 100
  labels:
    severity: critical

# Warning (Slack)
- alert: ElevatedP95Latency
  expr: histogram_quantile(0.95, ...) > 0.00005
  labels:
    severity: warning

# Info (Email)
- alert: PolicyDeployment
  labels:
    severity: info
```

---

## 📚 Resources

### Documentation

- [OpenTelemetry](https://opentelemetry.io/docs/)
- [Prometheus](https://prometheus.io/docs/)
- [Grafana](https://grafana.com/docs/)
- [Loki](https://grafana.com/docs/loki/latest/)
- [Tempo](https://grafana.com/docs/tempo/latest/)
- [OBSERVABILITY_STACK_GUIDE.md](./OBSERVABILITY_STACK_GUIDE.md) - Complete guide

### Example Queries

See [METRICS_GUIDE.md](./METRICS_GUIDE.md) for 50+ example PromQL queries

### Troubleshooting

**No traces appearing?**
```bash
# Check Tempo is receiving data
curl http://localhost:3200/api/search | jq

# Check OTel endpoint
docker logs reaper-agent | grep -i "tempo\|otel"
```

**Logs not showing in Loki?**
```bash
# Check Promtail is shipping logs
docker logs reaper-promtail

# Query Loki directly
curl -G http://localhost:3100/loki/api/v1/query \
  --data-urlencode 'query={service="reaper-agent"}'
```

**Metrics missing?**
```bash
# Check Prometheus targets
open http://localhost:9090/targets

# Check agent metrics endpoint
curl http://localhost:8080/metrics | grep reaper
```

---

## 🎯 What's Next?

### Immediate

✅ **You have production-grade observability NOW**
- Metrics: Prometheus
- Logs: Loki
- Traces: Tempo
- Dashboards: Grafana

### Short-term Enhancements

1. **Create Performance Dashboard** - Latency P50/P95/P99 trends
2. **Create Compliance Dashboard** - Audit trails for security teams
3. **Set up Alertmanager** - Route alerts to Slack/PagerDuty
4. **Add Grafana OnCall** - Escalation policies

### Long-term

1. **Machine Learning** - Anomaly detection with Prophet/ARIMA
2. **Cost Optimization** - Sampling strategies for high volume
3. **Multi-region** - Federated Prometheus, global view
4. **Event Streaming** - Real-time analytics with Kafka/NATS

---

## 🏆 Summary

You now have:

✅ **World-class observability** matching $31/host/month Datadog
✅ **Zero vendor lock-in** via OpenTelemetry
✅ **Complete visibility** into every policy decision
✅ **Sub-microsecond precision** tracking
✅ **Unified correlation** between logs, metrics, and traces
✅ **Production-ready** with Docker Compose
✅ **Kubernetes-ready** (use Helm charts)
✅ **Datadog-compatible** (optional dual export)

**Cost**: $0 (infrastructure only)
**Setup Time**: 5 minutes
**Capabilities**: Matches commercial APM solutions
**Lock-in**: Zero

---

*Last updated: 2025-12-14*
*Reaper Policy Engine - World-Class Observability, Zero Cost*
