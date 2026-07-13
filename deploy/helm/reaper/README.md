# Reaper Helm Chart

A Helm chart for deploying Reaper - a high-performance policy enforcement platform with sub-microsecond latency.

## Prerequisites

- Kubernetes 1.23+
- Helm 3.8+
- PV provisioner support (for persistence)

## Installation

### Add the repository

```bash
helm repo add reaper https://pgste.github.io/reaper/charts
helm repo update
```

### Install the chart

```bash
# Install with default values
helm install reaper reaper/reaper

# Install in a specific namespace
helm install reaper reaper/reaper -n reaper --create-namespace

# Install with custom values
helm install reaper reaper/reaper -f values.yaml
```

### Install from OCI registry

```bash
helm install reaper oci://ghcr.io/pgste/reaper/charts/reaper --version 0.1.0
```

## Deployment models

The chart ships ready-made **profiles** under [`profiles/`](profiles/) so you can
pick a shape instead of hand-assembling `enabled` flags. Two audiences:

- **Give to consumers** — a self-contained enforcement agent they run themselves.
- **Managed stack** — the multi-tenant control plane *you* operate.

| Profile | For | What's deployed | Install |
|---------|-----|-----------------|---------|
| `engine` | consumers | Agent only (HTTP enforcement) | `-f profiles/engine.yaml` |
| `engine-uds-sharded` | consumers | Agent only + thread-per-core UDS (highest throughput) | `-f profiles/engine-uds-sharded.yaml` |
| `platform` | single team | Agent + Platform (basic mgmt, no DB) | `-f profiles/platform.yaml` |
| `managed-stack` | **you (SaaS)** | Management + PostgreSQL + managed agents + audit pipeline (ClickHouse/Vector) | `-f profiles/managed-stack.yaml` |
| `full` | demo/staging | Everything + metrics + audit pipeline | `-f profiles/full.yaml` |

The `managed-stack` and `full` profiles enable the **decision-log audit
pipeline** (`decisionLogs.enabled=true`): every agent pod writes decision
NDJSON to a pod-shared file, a Vector sidecar ships it (disk WAL, end-to-end
acks) into a bundled single-node ClickHouse, and the management query API
(`GET /api/v1/orgs/{org}/decisions`, `/stats`, `/timeseries`, `/facets`,
`/{id}`) is auto-wired to that store. Required at install:
`--set decisionLogs.clickhouse.password=$(openssl rand -hex 24)`. To bring
your own ClickHouse/ClickHouse Cloud set `decisionLogs.clickhouse.enabled=false`
and `decisionLogs.clickhouse.url` (+ credentials via
`decisionLogs.clickhouse.existingSecret`). Multi-tenant: set
`decisionLogs.tenantId` to the org UUID the agents serve. A **privacy posture
is required** when decision logging is enabled (the agent refuses to start
otherwise): `decisionLogs.privacy=pseudonymize` (GDPR-friendly — principal and
resource pseudonymized; needs `REAPER_DECISION_LOG_HASH_SALT` in
`protectionExistingSecret`) or `privacy=raw` (explicit opt-out). Fine-grained
data protection (masking/pseudonymization/encryption) via
`decisionLogs.hashPrincipal`, `hashResource`, `maskKeys`, `encryptInputData` +
`protectionExistingSecret` (generate secrets with `reaper-cli decisions keygen`).

```bash
# Consumer: drop-in enforcement agent
helm install reaper ./deploy/helm/reaper -f ./deploy/helm/reaper/profiles/engine.yaml

# You: the managed multi-tenant control plane
helm install reaper ./deploy/helm/reaper -f ./deploy/helm/reaper/profiles/managed-stack.yaml \
    --set management.secrets.jwtSecret=$(openssl rand -hex 32) \
    --set postgresql.auth.password=$(openssl rand -hex 24)
```

### Sidecar (UDS) for consumers

To run the agent as a **sidecar** next to a consumer app and talk over a Unix
domain socket (no network hop), see
[`deploy/kubernetes/agent-sidecar-example.yaml`](../../kubernetes/agent-sidecar-example.yaml)
and [`docs/deployment/UDS_DEPLOYMENT.md`](../../../docs/deployment/UDS_DEPLOYMENT.md).
Enable UDS on any agent via `agent.uds.*` (see parameters below); pick the
**shared** model (`shards: 0`) for best tail latency or the **sharded**
thread-per-core model (`shards: N`) for peak throughput.

## Architecture

The chart deploys the following components:

| Component | Description |
|-----------|-------------|
| **Management Server** | Centralized policy management, organization management, bundle compilation |
| **Platform** | Legacy policy management (optional) |
| **Agent** | Policy enforcement engine (standalone or managed mode) |
| **PostgreSQL** | Database for management server (optional, uses Bitnami subchart) |

## Configuration

### Minimal Production Configuration

```yaml
# values-production.yaml
management:
  replicaCount: 3
  secrets:
    jwtSecret: "your-secure-jwt-secret-here"
  resources:
    requests:
      cpu: 200m
      memory: 256Mi
    limits:
      cpu: 1000m
      memory: 1Gi

agent:
  standalone:
    replicaCount: 5
  resources:
    requests:
      cpu: 100m
      memory: 128Mi
    limits:
      cpu: 500m
      memory: 512Mi

postgresql:
  auth:
    password: "your-secure-db-password"
```

### Enable Managed Mode

```yaml
agent:
  standalone:
    enabled: false
  managed:
    enabled: true
    replicaCount: 5
    organization: "my-org"
    apiKey: "your-api-key"
```

### Enable Ingress

```yaml
management:
  ingress:
    enabled: true
    className: nginx
    hosts:
      - host: management.example.com
        paths:
          - path: /
            pathType: Prefix
    tls:
      - secretName: reaper-tls
        hosts:
          - management.example.com

agent:
  ingress:
    enabled: true
    className: nginx
    hosts:
      - host: agent.example.com
        paths:
          - path: /
            pathType: Prefix
```

### Use External Database

```yaml
postgresql:
  enabled: false

externalDatabase:
  url: "postgres://user:password@host:5432/database"
```

### Enable ServiceMonitor for Prometheus

```yaml
metrics:
  serviceMonitor:
    enabled: true
    namespace: monitoring
    interval: 15s
    labels:
      release: prometheus
```

## Parameters

### Global Parameters

| Parameter | Description | Default |
|-----------|-------------|---------|
| `global.imageRegistry` | Global Docker image registry | `""` |
| `global.imagePullSecrets` | Global Docker registry secret names | `[]` |
| `global.storageClass` | Global StorageClass for PVCs | `""` |

### Management Server Parameters

| Parameter | Description | Default |
|-----------|-------------|---------|
| `management.enabled` | Enable Management Server | `true` |
| `management.replicaCount` | Number of replicas | `2` |
| `management.image.repository` | Image repository | `ghcr.io/pgste/reaper/reaper-management` |
| `management.image.tag` | Image tag | `""` (uses Chart.appVersion) |
| `management.service.type` | Service type | `ClusterIP` |
| `management.service.port` | Service port | `3000` |
| `management.secrets.jwtSecret` | JWT secret (generate random if empty) | `""` |
| `management.persistence.enabled` | Enable persistence | `true` |
| `management.persistence.size` | PVC size | `10Gi` |
| `management.autoscaling.enabled` | Enable HPA | `true` |
| `management.autoscaling.minReplicas` | Minimum replicas | `2` |
| `management.autoscaling.maxReplicas` | Maximum replicas | `10` |

### Agent Parameters

| Parameter | Description | Default |
|-----------|-------------|---------|
| `agent.standalone.enabled` | Enable standalone agent | `true` |
| `agent.standalone.replicaCount` | Number of replicas | `3` |
| `agent.managed.enabled` | Enable managed agent | `false` |
| `agent.managed.managementUrl` | Management server URL | `""` |
| `agent.managed.organization` | Organization slug | `default` |
| `agent.managed.apiKey` | API key for authentication | `""` |
| `agent.image.repository` | Image repository | `ghcr.io/pgste/reaper/reaper-agent` |
| `agent.autoscaling.enabled` | Enable HPA | `true` |
| `agent.autoscaling.minReplicas` | Minimum replicas | `3` |
| `agent.autoscaling.maxReplicas` | Maximum replicas | `20` |
| `agent.uds.enabled` | Serve over a Unix domain socket (adds a pod-local emptyDir) | `false` |
| `agent.uds.shards` | `0`/`1` = shared socket; `N>1` = sharded thread-per-core | `0` |
| `agent.uds.pinCores` | Pin each shard runtime to a core (needs Guaranteed QoS + static CPU manager) | `true` |
| `agent.uds.socketDir` | Socket directory (mounted as emptyDir) | `/run/reaper` |
| `agent.uds.socketName` | Socket file name (base name in sharded mode) | `agent.sock` |
| `agent.uds.permissions` | Socket file mode (octal string) | `"0660"` |

### PostgreSQL Parameters (Bitnami subchart)

| Parameter | Description | Default |
|-----------|-------------|---------|
| `postgresql.enabled` | Enable PostgreSQL | `true` |
| `postgresql.auth.username` | Database username | `reaper` |
| `postgresql.auth.database` | Database name | `reaper_management` |
| `postgresql.primary.persistence.size` | PVC size | `20Gi` |

## Upgrading

### To 0.2.0

Breaking changes:
- Management server now requires JWT secret to be set
- Agent configuration structure changed

## Uninstallation

```bash
helm uninstall reaper -n reaper
```

To also delete PVCs:

```bash
kubectl delete pvc -n reaper -l app.kubernetes.io/instance=reaper
```

## Troubleshooting

### Check pod status

```bash
kubectl get pods -n reaper -l app.kubernetes.io/instance=reaper
```

### View logs

```bash
kubectl logs -n reaper -l app.kubernetes.io/component=management
kubectl logs -n reaper -l app.kubernetes.io/component=agent
```

### Check health

```bash
kubectl port-forward svc/reaper-management 3000:3000 -n reaper
curl http://localhost:3000/health
```

## License

Apache 2.0
