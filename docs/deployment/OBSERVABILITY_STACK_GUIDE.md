# Reaper Observability Stack Guide

**Complete guide to vendor-neutral, world-class observability**

---

## 🎯 The Modern Observability Landscape

### Commercial vs Open Source

| Feature | Commercial (Datadog) | Open Source (Grafana Stack) | Hybrid (OTel) |
|---------|---------------------|----------------------------|---------------|
| **Cost** | ~$15-31/host/month | Free (infrastructure only) | Free instrumentation, choose backend |
| **Setup** | Minimal (SaaS) | Self-hosted (Docker/K8s) | Flexible |
| **Vendor Lock-in** | High | None | None |
| **Features** | Rich out-of-box | Highly customizable | Best of both |
| **Data Retention** | Limited by plan | You control | You control |
| **Best For** | Quick start, managed | Cost-sensitive, custom needs | Future-proof, flexibility |

---

## 🏆 The Reaper Approach: OpenTelemetry + Grafana Stack

**We use OpenTelemetry as the instrumentation layer**, which means:

✅ **Instrument once, export anywhere**
- Switch backends without code changes
- Send data to multiple backends simultaneously
- Future-proof against vendor changes

✅ **Best-in-class open source stack (default)**
- Prometheus (metrics)
- Grafana (visualization)
- Loki (logs)
- Tempo (traces)
- Equals or exceeds Datadog capabilities

✅ **Optional commercial integrations**
- Datadog exporter (if you want managed service)
- Honeycomb, New Relic, Lightstep, etc.

---

## 📊 The Three Pillars of Observability

### 1. Metrics (Prometheus)

**What**: Time-series numerical data
**Use Cases**: Performance monitoring, alerting, capacity planning

**Reaper Metrics**:
```prometheus
reaper_decisions_total{decision="allow",policy="rbac"} 15234
reaper_decision_duration_seconds_bucket{le="0.000001"} 14500
reaper_denials_total{resource="/api/secrets/42"} 12
```

**Best Practices**:
- High cardinality labels (policy_name, resource) for rich filtering
- Sub-microsecond histogram buckets for latency tracking
- Counter + Histogram + Gauge pattern

### 2. Logs (Loki)

**What**: Structured event data with context
**Use Cases**: Debugging, audit trails, security forensics

**Reaper Structured Logs** (JSON):
```json
{
  "timestamp": "2025-12-14T13:45:23.123456Z",
  "level": "warn",
  "target": "reaper_agent",
  "fields": {
    "message": "Access denied",
    "decision_id": "dec_a1b2c3",
    "policy_name": "rbac-prod",
    "principal": "user_123",
    "resource": "/api/secrets/42",
    "action": "read",
    "decision": "deny",
    "latency_ns": 1234
  }
}
```

**Best Practices**:
- Structured JSON format (never plain text)
- Include correlation IDs (trace_id, span_id, decision_id)
- Log levels: ERROR (always), WARN (denials), INFO (sampled), DEBUG (dev only)
- Sampling: 1% for allows, 100% for denials

### 3. Traces (Tempo + OpenTelemetry)

**What**: Distributed request flows across services
**Use Cases**: Performance debugging, dependency mapping, bottleneck identification

**Reaper Trace Structure**:
```
Request [trace_id: abc123]
  └─ API Gateway [span: api_request] 45µs
      └─ Auth Service [span: auth_check] 23µs
          └─ Reaper Agent [span: policy_eval] 1.2µs ← THIS IS US
              ├─ Load Policy [span: policy_load] 0.1µs
              ├─ Evaluate Rules [span: rule_eval] 0.8µs
              └─ Record Decision [span: decision_record] 0.3µs
```

**Best Practices**:
- Propagate trace context (W3C Trace Context standard)
- Sub-microsecond span precision
- Rich span attributes (policy_id, resource, action, decision)
- Automatic instrumentation where possible

---

## 🚀 Reaper's Complete Observability Architecture

### Architecture Diagram

```
┌─────────────────────────────────────────────────────────────────┐
│                      REAPER AGENT                                │
│                                                                   │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │  Policy Evaluation (with OpenTelemetry)                   │  │
│  │                                                            │  │
│  │  1. Start Span (policy_eval)                             │  │
│  │  2. Record Prometheus Metrics                            │  │
│  │  3. Emit Structured JSON Log                             │  │
│  │  4. End Span with attributes                             │  │
│  └──────────────────────────────────────────────────────────┘  │
│                           ↓ ↓ ↓                                 │
└───────────────────────────┼─┼─┼─────────────────────────────────┘
                            │ │ │
                ┌───────────┘ │ └───────────┐
                ↓             ↓             ↓
         [Prometheus]    [Loki]        [Tempo]
         (Metrics)       (Logs)        (Traces)
                ↓             ↓             ↓
         ┌──────────────────────────────────────┐
         │          GRAFANA                      │
         │  (Unified Observability Dashboard)    │
         │                                       │
         │  • Metrics charts                     │
         │  • Log explorer with trace correlation│
         │  • Distributed tracing UI             │
         │  • Alerts & notifications             │
         └──────────────────────────────────────┘
```

### Data Flow

1. **Policy Decision Occurs**:
   - Create OpenTelemetry span
   - Record Prometheus metrics
   - Emit structured JSON log
   - Add trace context to response

2. **Collection**:
   - Prometheus scrapes `/metrics` endpoint (5s interval)
   - Promtail ships logs to Loki
   - OTel Collector sends traces to Tempo

3. **Visualization**:
   - Grafana queries all three backends
   - Correlates logs/traces via trace_id
   - Presents unified view

4. **Alerting**:
   - Prometheus evaluates alert rules (30s interval)
   - Triggers Alertmanager
   - Sends to PagerDuty/Slack/Email

---

## 🛠️ Open Source "Datadog-like" Stack

### Components

1. **Grafana** - Unified visualization (replaces Datadog UI)
2. **Prometheus** - Metrics storage (replaces Datadog Metrics)
3. **Loki** - Log aggregation (replaces Datadog Logs)
4. **Tempo** - Distributed tracing (replaces Datadog APM)
5. **Alertmanager** - Alert routing (replaces Datadog Monitors)
6. **OpenTelemetry Collector** - Telemetry pipeline (vendor-neutral)

### Comparison Matrix

| Feature | Datadog | Grafana Stack | Winner |
|---------|---------|---------------|--------|
| **Metrics** | Datadog Metrics | Prometheus | 🟰 Tie |
| **Logs** | Datadog Logs | Loki | 🟰 Tie |
| **Traces** | Datadog APM | Tempo | 🟰 Tie |
| **Dashboards** | Datadog Dashboards | Grafana | 🏆 Grafana (more flexible) |
| **Alerting** | Datadog Monitors | Alertmanager | 🏆 Grafana (more powerful) |
| **Cost** | ~$15-31/host/mo | $0 (infra only) | 🏆 Grafana |
| **Setup Time** | 5 minutes (SaaS) | 30 minutes (Docker) | 🏆 Datadog |
| **Data Retention** | 15 days (limited) | Unlimited (you control) | 🏆 Grafana |
| **Customization** | Limited | Unlimited | 🏆 Grafana |
| **Vendor Lock-in** | High | None | 🏆 Grafana |

### Deployment

**Docker Compose** (dev/staging):
```bash
docker-compose -f docker-compose.observability.yml up -d
```

**Kubernetes** (production):
```bash
helm install grafana-stack grafana/grafana
helm install prometheus prometheus-community/kube-prometheus-stack
helm install loki grafana/loki-stack
helm install tempo grafana/tempo
```

**Managed Options** (if you want hosted open source):
- Grafana Cloud (free tier: 10K metrics, 50GB logs, 50GB traces)
- AWS Managed Prometheus + Grafana
- GCP Managed Prometheus + Grafana

---

## 🔌 Optional: Datadog Integration

If you want to use Datadog alongside or instead of Grafana Stack:

### Option 1: OTel → Datadog Exporter

**Add to Agent**:
```toml
[dependencies]
opentelemetry-datadog = "0.11"
```

**Configure OTel Collector**:
```yaml
exporters:
  datadog:
    api:
      key: ${DD_API_KEY}
      site: datadoghq.com

service:
  pipelines:
    traces:
      exporters: [datadog]
    metrics:
      exporters: [datadog]
```

### Option 2: Datadog Agent Sidecar

**Docker Compose**:
```yaml
services:
  datadog-agent:
    image: datadog/agent:latest
    environment:
      - DD_API_KEY=${DD_API_KEY}
      - DD_SITE=datadoghq.com
      - DD_PROMETHEUS_SCRAPE_ENABLED=true
      - DD_LOGS_ENABLED=true
      - DD_APM_ENABLED=true
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock:ro
      - /proc/:/host/proc/:ro
      - /sys/fs/cgroup/:/host/sys/fs/cgroup:ro
```

**Kubernetes**:
```bash
helm install datadog datadog/datadog \
  --set datadog.apiKey=$DD_API_KEY \
  --set datadog.apm.enabled=true \
  --set datadog.logs.enabled=true
```

### Option 3: Dual Export (Best of Both)

Send data to **both** Grafana Stack and Datadog:

```yaml
# OTel Collector config
exporters:
  # Open source
  otlp/tempo:
    endpoint: tempo:4317
  prometheus:
    endpoint: prometheus:9090
  loki:
    endpoint: http://loki:3100/loki/api/v1/push

  # Commercial
  datadog:
    api:
      key: ${DD_API_KEY}

service:
  pipelines:
    traces:
      exporters: [otlp/tempo, datadog]  # Both!
    metrics:
      exporters: [prometheus, datadog]  # Both!
```

**Benefits**:
- Grafana Stack for cost-effective long-term storage
- Datadog for managed service and ML-powered insights
- Zero vendor lock-in

---

## 📈 Best Practices & Standards

### 1. Use OpenTelemetry Semantic Conventions

**Don't invent your own attribute names**:
```rust
// ❌ Bad (custom names)
span.set_attribute("decision", "allow");
span.set_attribute("policy", "rbac");

// ✅ Good (OTel semantic conventions)
span.set_attribute("reaper.decision", "allow");
span.set_attribute("reaper.policy.name", "rbac");
span.set_attribute("http.status_code", 200);
```

**Resources**:
- [OTel Semantic Conventions](https://opentelemetry.io/docs/specs/semconv/)
- Reaper custom namespace: `reaper.*`

### 2. Correlation IDs Everywhere

**Link logs, traces, and metrics**:
```json
{
  "trace_id": "abc123",      // OpenTelemetry trace ID
  "span_id": "def456",       // OpenTelemetry span ID
  "decision_id": "dec_xyz",  // Reaper decision ID
  "request_id": "req_789"    // API gateway request ID
}
```

**Enables**:
- Click trace ID in logs → jump to trace
- Click log in trace → see full context
- Filter metrics by trace_id

### 3. Sampling Strategy

**High Volume Systems** (>1000 RPS):

| Event Type | Sample Rate | Reasoning |
|------------|-------------|-----------|
| Allow decisions | 1% | Reduce cost, still statistically significant |
| Deny decisions | 100% | Security critical, always log |
| Errors | 100% | Debugging critical |
| Slow requests (>100µs) | 100% | Performance investigation |
| Traces (head-based) | 10% | Enough for debugging |
| Traces (tail-based) | 100% of errors | Keep failed traces |

**Implementation**:
```rust
let sample_rate = if decision == "deny" || latency_us > 100.0 {
    1.0  // 100%
} else {
    0.01  // 1%
};

if should_sample(sample_rate) {
    log::info!("Decision: {}", decision);
}
```

### 4. Metric Naming Conventions

**Follow Prometheus best practices**:
```
<namespace>_<subsystem>_<name>_<unit>

Examples:
reaper_decisions_total              // Counter
reaper_decision_duration_seconds    // Histogram
reaper_active_policies              // Gauge
reaper_cache_hit_ratio              // Gauge (0.0-1.0)
```

**Units**:
- Time: `_seconds` (not ms or µs)
- Bytes: `_bytes` (not kb or mb)
- Ratio: `_ratio` (0.0-1.0, not percentage)

### 5. Dashboard Design Principles

**Use the RED method**:
- **R**ate: Requests per second
- **E**rrors: Error rate
- **D**uration: Latency distribution (P50, P95, P99)

**Use the USE method** (for resources):
- **U**tilization: % resource used
- **S**aturation: Queue depth
- **E**rrors: Error count

**Dashboard Structure**:
1. **Top Row**: KPIs (SLIs) - Must be green for system to be healthy
2. **Second Row**: Trends - Is it getting better or worse?
3. **Third Row**: Details - Drill-down for investigation

---

## 🎨 Reaper Observability Features

### Implemented

✅ **Prometheus Metrics** (Phase 8.2)
- 8 core metrics with rich labels
- Sub-microsecond histogram buckets
- `/metrics` endpoint in Prometheus format

✅ **Grafana Security Dashboard** (Phase 8.2)
- 10 panels with 5s refresh
- Live denial stream
- Suspicious activity detection

✅ **Prometheus Alert Rules** (Phase 8.2)
- 20+ alert rules across 5 categories
- Security, performance, availability, compliance, capacity

✅ **Docker Compose Stack** (Phase 8.2)
- One-command deployment
- Prometheus + Grafana + Loki + Promtail

### Coming Next

🔄 **Structured JSON Logging** (Phase 8.3)
- Replace plain text logs with JSON
- Include trace_id for correlation
- Sampling for high volume

🔄 **OpenTelemetry Tracing** (Phase 8.3)
- Distributed tracing across services
- Sub-microsecond span precision
- Export to Tempo (default) or Datadog/Jaeger

🔄 **Additional Grafana Dashboards** (Phase 8.3)
- Performance Dashboard (latency, cache, throughput)
- Compliance Dashboard (audit trails, access patterns)
- Developer Dashboard (policy debugging)
- Executive Dashboard (KPIs, trends)

🔄 **OpenTelemetry Collector** (Phase 8.3)
- Central telemetry pipeline
- Multi-backend export (Tempo + Datadog)
- Tail-based sampling

---

## 📚 Resources

### Documentation

- [OpenTelemetry Docs](https://opentelemetry.io/docs/)
- [Prometheus Best Practices](https://prometheus.io/docs/practices/)
- [Grafana Tutorials](https://grafana.com/tutorials/)
- [Loki Documentation](https://grafana.com/docs/loki/latest/)
- [Tempo Documentation](https://grafana.com/docs/tempo/latest/)

### Alternatives & Integrations

**Other Open Source Stacks**:
- ELK Stack (Elasticsearch + Logstash + Kibana) - Heavy, powerful
- ClickHouse + Grafana - Ultra-fast analytics
- VictoriaMetrics + Grafana - Prometheus-compatible, faster

**Commercial Alternatives to Datadog**:
- Honeycomb - Traces-first, great for debugging
- New Relic - APM-focused
- Lightstep - Enterprise observability
- Dynatrace - Auto-instrumentation
- Splunk - Log-focused, enterprise

**Hybrid/Managed Open Source**:
- Grafana Cloud - Hosted Grafana Stack
- Coralogix - Managed ELK alternative
- Observe - Data lake-based observability

---

## 🎯 Recommendation: Start with Grafana Stack

**For most users, we recommend:**

1. **Development/Staging**: Docker Compose Grafana Stack (free)
2. **Production**:
   - Small (<10 services): Self-hosted Grafana Stack on K8s
   - Medium (10-100 services): Grafana Cloud free tier → paid
   - Large (100+ services): Grafana Cloud or Datadog

**Why?**
- Zero cost to start
- Learn observability fundamentals
- No vendor lock-in
- Can always migrate to Datadog later (via OTel)
- Better customization

**When to use Datadog:**
- You need managed service (no ops team)
- Budget allows (~$15-31/host/month)
- Want ML-powered anomaly detection
- Need enterprise support SLAs

---

*Last updated: 2025-12-14*
*Reaper Policy Engine - World-Class Observability*
