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
