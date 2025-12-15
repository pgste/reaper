# Reaper Prometheus Metrics Guide

Complete guide to the Prometheus metrics emitted by Reaper Agent for real-time decision monitoring.

---

## 📊 Overview

Every policy decision generates structured metrics that flow into Prometheus in real-time. These metrics power:
- **Security Dashboards** - Live denial streams and attack detection
- **Performance Monitoring** - Sub-microsecond latency tracking
- **Compliance Reporting** - Audit trails and access patterns
- **Alerting** - Automated notifications for security events

---

## 🔢 Available Metrics

### Decision Metrics

#### `reaper_decisions_total` (Counter)
Total policy decisions made by outcome.

**Labels**:
- `decision` - Decision outcome: `allow`, `deny`, `log`
- `policy_name` - Name of the policy that made the decision
- `policy_id` - Unique policy identifier (UUID)

**Example**:
```prometheus
reaper_decisions_total{decision="allow",policy_name="demo-allow-all",policy_id="..."} 1523
reaper_decisions_total{decision="deny",policy_name="rbac-prod",policy_id="..."} 47
```

**Use Cases**:
- Track total decisions over time
- Calculate allow vs deny ratios
- Identify most-used policies

#### `reaper_decision_duration_seconds` (Histogram)
Policy decision latency distribution in seconds (sub-microsecond precision).

**Labels**:
- `policy_name` - Name of the policy evaluated

**Buckets**: 100ns, 500ns, 1µs, 5µs, 10µs, 50µs, 100µs, 500µs, 1ms

**Example**:
```prometheus
reaper_decision_duration_seconds_bucket{policy_name="rbac-prod",le="0.000001"} 1450
reaper_decision_duration_seconds_bucket{policy_name="rbac-prod",le="0.00001"} 1523
reaper_decision_duration_seconds_sum{policy_name="rbac-prod"} 0.001834
reaper_decision_duration_seconds_count{policy_name="rbac-prod"} 1523
```

**Use Cases**:
- Calculate P50, P95, P99 latency
- Detect performance degradation
- Identify slow policies
- Track sub-microsecond performance goals

#### `reaper_denials_total` (Counter)
Total policy denials for security monitoring.

**Labels**:
- `policy_name` - Name of the policy that denied access
- `resource` - Resource that was denied access
- `action` - Action that was denied (read, write, delete, etc.)

**Example**:
```prometheus
reaper_denials_total{policy_name="confidential-access",resource="/api/secrets/42",action="read"} 12
```

**Use Cases**:
- Security event monitoring
- Brute force attack detection
- Failed access attempt tracking
- Compliance audit trails

---

### Cache Metrics

#### `reaper_cache_hits_total` (Counter)
Cache hit count by cache type.

**Labels**:
- `cache_type` - Type of cache: `policy`, `regex`, `attribute`

**Example**:
```prometheus
reaper_cache_hits_total{cache_type="policy"} 45623
```

#### `reaper_cache_misses_total` (Counter)
Cache miss count by cache type.

**Labels**:
- `cache_type` - Type of cache: `policy`, `regex`, `attribute`

**Example**:
```prometheus
reaper_cache_misses_total{cache_type="policy"} 234
```

**Use Cases**:
- Calculate cache hit rate
- Optimize cache configuration
- Detect cache thrashing

---

### System Metrics

#### `reaper_active_policies` (Gauge)
Number of currently loaded policies.

**Example**:
```prometheus
reaper_active_policies 5
```

**Use Cases**:
- Track policy deployments
- Ensure policies are loaded
- Alert on zero policies

#### `reaper_concurrent_evaluations` (Gauge)
Current number of concurrent policy evaluations.

**Example**:
```prometheus
reaper_concurrent_evaluations 23
```

**Use Cases**:
- Monitor system load
- Capacity planning
- Detect traffic spikes

#### `reaper_errors_total` (Counter)
Total errors during policy evaluation.

**Labels**:
- `error_type` - Error classification:
  - `invalid_policy_id` - Invalid UUID format
  - `policy_not_found` - Policy lookup failed
  - `no_policies` - No policies available
  - `evaluation_error` - Policy evaluation failed
  - `policy_deployment_failed` - Policy deployment failed

**Example**:
```prometheus
reaper_errors_total{error_type="policy_not_found"} 5
```

**Use Cases**:
- Track error rates
- Alert on evaluation failures
- Debugging production issues

---

## 📈 Common PromQL Queries

### Throughput

```promql
# Decisions per second (5-minute rate)
rate(reaper_decisions_total[5m])

# Allow decisions per second
rate(reaper_decisions_total{decision="allow"}[5m])

# Deny decisions per second
rate(reaper_decisions_total{decision="deny"}[5m])

# Total requests per second
sum(rate(reaper_decisions_total[5m]))
```

### Latency

```promql
# P50 latency
histogram_quantile(0.50, rate(reaper_decision_duration_seconds_bucket[5m]))

# P95 latency
histogram_quantile(0.95, rate(reaper_decision_duration_seconds_bucket[5m]))

# P99 latency (target: <100µs = 0.0001s)
histogram_quantile(0.99, rate(reaper_decision_duration_seconds_bucket[5m]))

# Average latency by policy
rate(reaper_decision_duration_seconds_sum[5m]) / rate(reaper_decision_duration_seconds_count[5m])

# Latency by policy name
histogram_quantile(0.99, rate(reaper_decision_duration_seconds_bucket{policy_name="rbac-prod"}[5m]))
```

### Security

```promql
# Denial rate
rate(reaper_denials_total[5m])

# Denial ratio (percentage)
100 * rate(reaper_denials_total[5m]) / rate(reaper_decisions_total[5m])

# Denials by resource
sum by (resource) (rate(reaper_denials_total[5m]))

# Top 10 denied resources
topk(10, sum by (resource) (increase(reaper_denials_total[1h])))

# Repeated denials from same resource (brute force detection)
increase(reaper_denials_total[1m]) > 10
```

### Cache Performance

```promql
# Cache hit rate
rate(reaper_cache_hits_total[5m]) /
  (rate(reaper_cache_hits_total[5m]) + rate(reaper_cache_misses_total[5m]))

# Cache hit rate percentage
100 * rate(reaper_cache_hits_total[5m]) /
  (rate(reaper_cache_hits_total[5m]) + rate(reaper_cache_misses_total[5m]))

# Cache misses per second
rate(reaper_cache_misses_total[5m])
```

### System Health

```promql
# Active policies
reaper_active_policies

# Concurrent evaluations
reaper_concurrent_evaluations

# Error rate
rate(reaper_errors_total[5m])

# Error rate by type
sum by (error_type) (rate(reaper_errors_total[5m]))
```

---

## 🚨 Alerting Rules

### Critical Alerts

```yaml
# High denial rate (potential attack)
- alert: HighDenialRate
  expr: rate(reaper_denials_total[5m]) > 100
  for: 2m
  labels:
    severity: critical
  annotations:
    summary: "High denial rate detected"
    description: "{{ $value }} denials/sec - possible attack"

# P99 latency degradation
- alert: HighP99Latency
  expr: histogram_quantile(0.99, rate(reaper_decision_duration_seconds_bucket[5m])) > 0.0001
  for: 5m
  labels:
    severity: critical
  annotations:
    summary: "P99 latency > 100µs"
    description: "Current P99: {{ $value }}s"

# Policy evaluation failures
- alert: PolicyEvaluationFailures
  expr: rate(reaper_errors_total[5m]) > 10
  for: 1m
  labels:
    severity: critical
  annotations:
    summary: "Policy evaluation failures detected"
```

### Warning Alerts

```yaml
# Low cache hit rate
- alert: LowCacheHitRate
  expr: |
    rate(reaper_cache_hits_total[10m]) /
    (rate(reaper_cache_hits_total[10m]) + rate(reaper_cache_misses_total[10m])) < 0.8
  for: 10m
  labels:
    severity: warning
  annotations:
    summary: "Cache hit rate below 80%"

# No policies loaded
- alert: NoPoliciesLoaded
  expr: reaper_active_policies == 0
  for: 5m
  labels:
    severity: warning
  annotations:
    summary: "No active policies loaded"
```

---

## 🔧 Accessing Metrics

### Metrics Endpoint

The Agent exposes Prometheus metrics at:

```
GET http://localhost:8080/metrics
```

**Response Format**: Prometheus text format

**Example**:
```bash
curl http://localhost:8080/metrics
```

**Output**:
```
# HELP reaper_decisions_total Total policy decisions made
# TYPE reaper_decisions_total counter
reaper_decisions_total{decision="allow",policy_name="demo-allow-all",policy_id="..."} 1523

# HELP reaper_decision_duration_seconds Policy decision latency in seconds
# TYPE reaper_decision_duration_seconds histogram
reaper_decision_duration_seconds_bucket{policy_name="rbac-prod",le="0.000001"} 1450
reaper_decision_duration_seconds_bucket{policy_name="rbac-prod",le="0.00001"} 1523
reaper_decision_duration_seconds_sum{policy_name="rbac-prod"} 0.001834
reaper_decision_duration_seconds_count{policy_name="rbac-prod"} 1523

# HELP reaper_active_policies Number of active policies loaded
# TYPE reaper_active_policies gauge
reaper_active_policies 5
```

### Prometheus Configuration

Configure Prometheus to scrape the Agent:

```yaml
scrape_configs:
  - job_name: 'reaper-agent'
    static_configs:
      - targets: ['agent:8080']
    metrics_path: '/metrics'
    scrape_interval: 5s  # High frequency for sub-microsecond tracking
```

### Testing Metrics

Use the provided test script:

```bash
./observability/test-metrics.sh
```

This script will:
1. Send test policy evaluations
2. Fetch Prometheus metrics
3. Display key metrics
4. Show example PromQL queries

---

## 📊 Grafana Integration

### Datasource Configuration

Add Prometheus as a datasource in Grafana:

1. Navigate to Configuration → Data Sources
2. Add new Prometheus datasource
3. URL: `http://prometheus:9090`
4. Save & Test

### Pre-built Dashboards

Import the included dashboards:

- **Security Dashboard**: `observability/grafana/dashboards/security-dashboard.json`
  - Live denial stream
  - Top denied users
  - Suspicious activity alerts
  - Decision rate trends

More dashboards coming soon:
- Performance Dashboard
- Compliance Dashboard
- Developer Dashboard
- Executive Dashboard

---

## 🎯 Metrics Best Practices

### 1. Set Appropriate Scrape Intervals

```yaml
# Agent: High-frequency for latency tracking
scrape_interval: 5s

# Platform: Lower frequency acceptable
scrape_interval: 10s
```

### 2. Use Recording Rules for Heavy Queries

```yaml
groups:
  - name: reaper_aggregations
    interval: 30s
    rules:
      # Pre-compute P99 latency
      - record: reaper:decision_latency:p99
        expr: histogram_quantile(0.99, rate(reaper_decision_duration_seconds_bucket[5m]))

      # Pre-compute cache hit rate
      - record: reaper:cache:hit_rate
        expr: |
          rate(reaper_cache_hits_total[5m]) /
          (rate(reaper_cache_hits_total[5m]) + rate(reaper_cache_misses_total[5m]))
```

### 3. Configure Retention

```yaml
# Prometheus retention
--storage.tsdb.retention.time=30d
--storage.tsdb.retention.size=50GB
```

### 4. Export Long-term Data

For compliance and historical analysis:
- Export to time-series database (InfluxDB, TimescaleDB)
- Store in data warehouse (BigQuery, Snowflake)
- Archive to S3/blob storage

---

## 🔍 Debugging with Metrics

### Finding Slow Policies

```promql
# Policies with P99 > 100µs
max by (policy_name) (
  histogram_quantile(0.99, rate(reaper_decision_duration_seconds_bucket[5m]))
) > 0.0001
```

### Identifying Cache Issues

```promql
# Policies with low cache hit rate
(
  rate(reaper_cache_hits_total{cache_type="policy"}[5m]) /
  (rate(reaper_cache_hits_total{cache_type="policy"}[5m]) +
   rate(reaper_cache_misses_total{cache_type="policy"}[5m]))
) < 0.8
```

### Detecting Anomalies

```promql
# Denial rate spike (2x higher than baseline)
rate(reaper_denials_total[5m]) > 2 * avg_over_time(rate(reaper_denials_total[5m])[1h:5m])
```

---

## 📚 Additional Resources

- [Prometheus Documentation](https://prometheus.io/docs/)
- [PromQL Cheat Sheet](https://promlabs.com/promql-cheat-sheet/)
- [Grafana Dashboard Best Practices](https://grafana.com/docs/grafana/latest/best-practices/)
- [OBSERVABILITY_VISION.md](./OBSERVABILITY_VISION.md) - Complete observability architecture

---

*Last updated: 2025-12-14*
