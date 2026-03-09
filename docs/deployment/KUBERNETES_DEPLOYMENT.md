# Kubernetes Deployment Guide

This guide covers deploying Reaper to Kubernetes using either Helm charts or raw manifests.

## Deployment Options

| Method | Best For | Complexity |
|--------|----------|------------|
| **Helm Chart** | Production, GitOps | Low |
| **Kustomize** | Customized deployments | Medium |
| **Raw Manifests** | Learning, testing | Medium |
| **Docker Compose** | Local development | Low |

## Prerequisites

- Kubernetes 1.23+
- kubectl configured
- Helm 3.8+ (for Helm deployment)
- Container registry access

## Quick Start with Helm

### 1. Install from OCI Registry

```bash
# Create namespace
kubectl create namespace reaper

# Install with defaults
helm install reaper oci://ghcr.io/pgste/reaper/charts/reaper \
  --namespace reaper \
  --version 0.1.0
```

### 2. Verify Installation

```bash
# Check pods
kubectl get pods -n reaper

# Check services
kubectl get svc -n reaper

# View installation notes
helm get notes reaper -n reaper
```

### 3. Access Services

```bash
# Management API
kubectl port-forward svc/reaper-management 3000:3000 -n reaper

# Agent API
kubectl port-forward svc/reaper-agent 8080:8080 -n reaper

# Verify health
curl http://localhost:3000/health
curl http://localhost:8080/health
```

## Production Deployment

### Recommended Configuration

```yaml
# values-production.yaml

# Management Server
management:
  replicaCount: 3

  secrets:
    # IMPORTANT: Set a secure JWT secret
    jwtSecret: "your-32-character-secure-secret"

  resources:
    requests:
      cpu: 200m
      memory: 256Mi
    limits:
      cpu: 1000m
      memory: 1Gi

  autoscaling:
    enabled: true
    minReplicas: 3
    maxReplicas: 10

  persistence:
    enabled: true
    size: 50Gi
    storageClass: "fast-ssd"

  ingress:
    enabled: true
    className: nginx
    annotations:
      cert-manager.io/cluster-issuer: letsencrypt-prod
    hosts:
      - host: management.reaper.example.com
        paths:
          - path: /
            pathType: Prefix
    tls:
      - secretName: reaper-management-tls
        hosts:
          - management.reaper.example.com

# Agent
agent:
  standalone:
    enabled: true
    replicaCount: 5

  resources:
    requests:
      cpu: 100m
      memory: 128Mi
    limits:
      cpu: 500m
      memory: 512Mi

  autoscaling:
    enabled: true
    minReplicas: 5
    maxReplicas: 50
    targetCPUUtilizationPercentage: 50

  podDisruptionBudget:
    enabled: true
    minAvailable: 3

# PostgreSQL
postgresql:
  enabled: true
  auth:
    password: "secure-database-password"
  primary:
    persistence:
      size: 100Gi
      storageClass: "fast-ssd"
  resources:
    requests:
      cpu: 500m
      memory: 512Mi
    limits:
      cpu: 2000m
      memory: 2Gi

# Monitoring
metrics:
  serviceMonitor:
    enabled: true
    namespace: monitoring
    labels:
      release: prometheus
```

### Deploy Production Configuration

```bash
helm install reaper oci://ghcr.io/pgste/reaper/charts/reaper \
  --namespace reaper \
  --create-namespace \
  -f values-production.yaml
```

## Deployment with Kustomize

### Directory Structure

```
deploy/kubernetes/
├── kustomization.yaml
├── namespace.yaml
├── postgres.yaml
├── reaper-management.yaml
├── reaper-platform.yaml
├── reaper-agent.yaml
└── ingress.yaml
```

### Deploy with Kustomize

```bash
# Preview resources
kubectl kustomize deploy/kubernetes/

# Apply to cluster
kubectl apply -k deploy/kubernetes/
```

### Create Overlays for Environments

```bash
mkdir -p deploy/kubernetes/overlays/{dev,staging,prod}
```

**Production Overlay:**

```yaml
# deploy/kubernetes/overlays/prod/kustomization.yaml
apiVersion: kustomize.config.k8s.io/v1beta1
kind: Kustomization

resources:
  - ../../

namespace: reaper-prod

replicas:
  - name: reaper-management
    count: 3
  - name: reaper-agent
    count: 5

images:
  - name: reaper-management
    newTag: v0.1.0
  - name: reaper-agent
    newTag: v0.1.0

patches:
  - path: resources-patch.yaml
```

## Architecture Patterns

### Standalone Mode

Agents connect directly to Platform for policy management:

```
┌─────────────┐     ┌──────────────┐     ┌─────────────┐
│   Client    │────▶│    Agent     │────▶│  Platform   │
│             │     │  (policy     │     │  (policy    │
│             │     │  evaluation) │     │  mgmt)      │
└─────────────┘     └──────────────┘     └─────────────┘
```

### Managed Mode

Agents connect to Management Server for centralized control:

```
┌─────────────┐     ┌──────────────┐     ┌─────────────┐
│   Client    │────▶│    Agent     │────▶│ Management  │
│             │     │  (managed)   │     │   Server    │
└─────────────┘     └──────────────┘     └──────┬──────┘
                                                 │
                                          ┌──────▼──────┐
                                          │ PostgreSQL  │
                                          └─────────────┘
```

### High Availability

```yaml
# Ensure HA with anti-affinity
agent:
  affinity:
    podAntiAffinity:
      preferredDuringSchedulingIgnoredDuringExecution:
        - weight: 100
          podAffinityTerm:
            labelSelector:
              matchLabels:
                app.kubernetes.io/component: agent
            topologyKey: kubernetes.io/hostname
```

## Observability

### Enable Prometheus Metrics

All services expose Prometheus metrics:

| Service | Endpoint | Port |
|---------|----------|------|
| Management | `/metrics/prometheus` | 3000 |
| Platform | `/metrics/prometheus` | 8081 |
| Agent | `/metrics` | 8080 |

### ServiceMonitor Configuration

```yaml
metrics:
  serviceMonitor:
    enabled: true
    namespace: monitoring
    interval: 15s
    scrapeTimeout: 10s
    labels:
      release: prometheus
```

### Grafana Dashboards

Import dashboards from `observability/grafana/dashboards/`:
- `reaper-overview.json` - System overview
- `reaper-agent.json` - Agent metrics
- `reaper-management.json` - Management metrics

## Security Best Practices

### 1. Secrets Management

Use external secrets operator or sealed secrets:

```yaml
management:
  secrets:
    existingSecret: "reaper-secrets"  # Use existing secret
```

### 2. Network Policies

```yaml
networkPolicy:
  enabled: true
  ingress:
    enabled: true
    namespaceSelector:
      matchLabels:
        name: ingress-nginx
  egress:
    enabled: true
    ports:
      - port: 5432  # PostgreSQL
        protocol: TCP
```

### 3. Pod Security Standards

All pods run with:
- Non-root user (UID 1000)
- Read-only root filesystem
- No privilege escalation
- Dropped capabilities

### 4. TLS Everywhere

```yaml
management:
  ingress:
    tls:
      - secretName: reaper-tls
        hosts:
          - management.example.com
```

## Troubleshooting

### Common Issues

**Pods not starting:**
```bash
kubectl describe pod -n reaper <pod-name>
kubectl logs -n reaper <pod-name> --previous
```

**Database connection issues:**
```bash
kubectl exec -n reaper -it <management-pod> -- env | grep DATABASE
kubectl logs -n reaper -l app.kubernetes.io/name=postgresql
```

**Health check failures:**
```bash
kubectl port-forward -n reaper svc/reaper-management 3000:3000
curl -v http://localhost:3000/health
```

### Debugging Commands

```bash
# Get all resources
kubectl get all -n reaper

# Describe deployment
kubectl describe deployment reaper-management -n reaper

# View events
kubectl get events -n reaper --sort-by='.lastTimestamp'

# Execute into pod
kubectl exec -n reaper -it <pod-name> -- /bin/sh
```

## Scaling

### Manual Scaling

```bash
kubectl scale deployment reaper-agent --replicas=10 -n reaper
```

### Autoscaling

HPA is enabled by default. Monitor with:

```bash
kubectl get hpa -n reaper
```

### Vertical Scaling

Adjust resources in values:

```yaml
agent:
  resources:
    requests:
      cpu: 200m
      memory: 256Mi
    limits:
      cpu: 1000m
      memory: 1Gi
```

## Upgrades

### Helm Upgrade

```bash
# Upgrade with new values
helm upgrade reaper oci://ghcr.io/pgste/reaper/charts/reaper \
  --namespace reaper \
  -f values-production.yaml

# Rollback if needed
helm rollback reaper 1 -n reaper
```

### Blue-Green Deployment

Use Argo Rollouts or Flagger for advanced deployment strategies.

## Cleanup

```bash
# Uninstall Helm release
helm uninstall reaper -n reaper

# Delete PVCs (data will be lost!)
kubectl delete pvc -n reaper -l app.kubernetes.io/instance=reaper

# Delete namespace
kubectl delete namespace reaper
```
