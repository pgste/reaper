# Docker Deployment Guide

**Phase 8.1: Production Deployment**

Quick start guide for deploying Reaper Policy Engine with Docker and Docker Compose.

---

## 🚀 Quick Start (5 minutes)

### Prerequisites

- Docker 24.0+ installed
- Docker Compose v2.0+ installed
- 4GB RAM minimum
- Linux/macOS/Windows (with WSL2)

### Deploy Full Stack

```bash
# Clone repository
cd /workspaces/reaper

# Build and start services
docker-compose up --build

# Or run in detached mode
docker-compose up --build -d
```

**Services will be available at**:
- **Agent**: http://localhost:8080
- **Platform**: http://localhost:8081

### Verify Deployment

```bash
# Check Agent health
curl http://localhost:8080/health

# Check Platform health
curl http://localhost:8081/health

# Check Agent readiness (should have policies loaded)
curl http://localhost:8080/ready

# View Agent metrics
curl http://localhost:8080/metrics

# View Platform metrics
curl http://localhost:8081/metrics
```

---

## 📦 Container Architecture

### Image Sizes (Optimized)

- **reaper-agent**: ~15MB (Alpine Linux + static binary)
- **reaper-platform**: ~15MB (Alpine Linux + static binary)

### Multi-Stage Build Benefits

1. **Builder Stage** (rust:1.75-alpine):
   - Compiles Rust binaries
   - Includes all build tools
   - ~1.5GB total size

2. **Runtime Stage** (alpine:3.19):
   - Contains only the binary
   - Minimal attack surface
   - ~15MB final size

### Security Features

- ✅ Non-root user (UID 1000)
- ✅ Minimal base image (Alpine)
- ✅ No shell in final image
- ✅ Read-only root filesystem capable
- ✅ Health checks built-in

---

## 🔍 Health Checks

### Kubernetes-Ready Probes

**Agent Endpoints**:
- `/health` - Basic liveness (always returns 200 OK)
- `/ready` - Readiness probe (checks policies loaded)
- `/live` - Liveness probe (checks process responsive)

**Platform Endpoints**:
- `/health` - Basic liveness
- *(Additional probes to be added)*

### Docker Compose Health Check

```yaml
healthcheck:
  test: ["CMD", "wget", "--quiet", "--tries=1", "--spider", "http://localhost:8080/health"]
  interval: 10s        # Check every 10 seconds
  timeout: 3s          # Fail if no response in 3s
  retries: 3           # Retry 3 times before marking unhealthy
  start_period: 10s    # Wait 10s before first check
```

### Check Container Health

```bash
# View health status
docker-compose ps

# View logs
docker-compose logs agent
docker-compose logs platform

# Follow logs
docker-compose logs -f agent
```

---

## 🎛️ Configuration

### Environment Variables

**Agent Configuration** (docker-compose.yml):
```yaml
environment:
  - RUST_LOG=info,reaper_agent=debug
  - REAPER_BIND_ADDR=0.0.0.0:8080
  - PLATFORM_URL=http://platform:8081
```

**Platform Configuration**:
```yaml
environment:
  - RUST_LOG=info,reaper_platform=debug
  - REAPER_BIND_ADDR=0.0.0.0:8081
```

### Volume Mounts

**Agent**:
```yaml
volumes:
  - agent-data:/data              # Persistent state
  - ./policies:/policies:ro        # Policy bundles (read-only)
```

**Platform**:
```yaml
volumes:
  - platform-data:/data            # Persistent policy store
```

---

## 📁 Policy Bundle Loading

### Pre-compile Policies to Bundles

```bash
# Create policies directory
mkdir -p policies

# Compile .reap to .rbb bundle (future CLI feature)
./target/release/reaper-cli compile my-policy.reap -o policies/my-policy.rbb

# Mount in docker-compose.yml
volumes:
  - ./policies:/policies:ro
```

### Agent Will Auto-Load

Agent automatically loads `.rbb` bundles from `/policies` directory on startup.

---

## 🌐 Networking

### Docker Network

**Network**: `reaper-network` (bridge mode)

Services communicate internally:
- Agent → Platform: `http://platform:8081`
- Platform → Agent: `http://agent:8080`

### Port Mapping

| Service | Internal | External | Purpose |
|---------|----------|----------|---------|
| Agent | 8080 | 8080 | Policy evaluation API |
| Platform | 8081 | 8081 | Policy management API |

### Firewall Rules

**Production deployment**:
- Agent (8080): Expose to application services
- Platform (8081): Restrict to admin network only

---

## 🔧 Operations

### Start Services

```bash
# Start all services
docker-compose up -d

# Start specific service
docker-compose up -d agent
docker-compose up -d platform
```

### Stop Services

```bash
# Stop all services
docker-compose down

# Stop and remove volumes
docker-compose down -v
```

### View Logs

```bash
# All logs
docker-compose logs

# Specific service
docker-compose logs agent
docker-compose logs platform

# Follow logs (tail -f)
docker-compose logs -f agent

# Last 100 lines
docker-compose logs --tail=100 agent
```

### Restart Services

```bash
# Restart all
docker-compose restart

# Restart specific service
docker-compose restart agent
```

### Scale Agent

```bash
# Run multiple agent replicas
docker-compose up --scale agent=3 -d

# Load balance with NGINX/HAProxy
```

---

## 🏗️ Building Images Manually

### Build Agent

```bash
# From repository root
docker build -f services/reaper-agent/Dockerfile -t reaper-agent:latest .

# Run standalone
docker run -p 8080:8080 reaper-agent:latest
```

### Build Platform

```bash
# From repository root
docker build -f services/reaper-platform/Dockerfile -t reaper-platform:latest .

# Run standalone
docker run -p 8081:8081 reaper-platform:latest
```

### Build Both

```bash
# Use docker-compose
docker-compose build

# Or build with no cache
docker-compose build --no-cache
```

---

## 🧪 Testing

### Test Policy Evaluation

```bash
# Create test policy
curl -X POST http://localhost:8081/api/v1/policies \
  -H "Content-Type: application/json" \
  -d '{
    "name": "test-policy",
    "description": "Test policy",
    "rules": [{
      "action": "allow",
      "resource": "/api/*",
      "conditions": []
    }]
  }'

# Deploy to agent
POLICY_ID="<policy-id-from-response>"
curl -X POST "http://localhost:8081/api/v1/policies/$POLICY_ID/deploy"

# Evaluate request
curl -X POST http://localhost:8080/api/v1/messages \
  -H "Content-Type: application/json" \
  -d '{
    "resource": "/api/users",
    "action": "GET",
    "context": {}
  }'
```

### Load Testing

```bash
# Install wrk
sudo apt-get install wrk

# Run load test (10 connections, 30 seconds)
wrk -t4 -c10 -d30s \
  -s post.lua \
  http://localhost:8080/api/v1/messages
```

---

## 🐛 Troubleshooting

### Agent Won't Start

**Check logs**:
```bash
docker-compose logs agent
```

**Common issues**:
- Port 8080 already in use
- Platform not healthy (agent depends on it)
- Insufficient memory

**Solutions**:
```bash
# Check port usage
sudo lsof -i :8080

# Restart platform first
docker-compose restart platform

# Check memory
docker stats
```

### Platform Won't Start

**Check logs**:
```bash
docker-compose logs platform
```

**Common issues**:
- Port 8081 already in use
- Volume permission issues

**Solutions**:
```bash
# Check port
sudo lsof -i :8081

# Fix permissions
sudo chown -R 1000:1000 /var/lib/docker/volumes/reaper-platform-data
```

### Health Checks Failing

**Symptoms**:
- Container marked as unhealthy
- Restarts continuously

**Debug**:
```bash
# Run health check manually
docker exec -it reaper-agent wget --quiet --tries=1 --spider http://localhost:8080/health

# Check if service is listening
docker exec -it reaper-agent netstat -tlnp
```

### Cannot Connect to Services

**Check network**:
```bash
# Inspect network
docker network inspect reaper-network

# Test connectivity
docker exec -it reaper-agent ping platform
docker exec -it reaper-platform ping agent
```

---

## 📊 Monitoring

### Prometheus Metrics (Future)

Agent and Platform will expose Prometheus metrics at `/metrics`:

```bash
# Scrape metrics
curl http://localhost:8080/metrics
```

**Key metrics**:
- `reaper_policy_evaluations_total`
- `reaper_policy_evaluation_duration_seconds`
- `reaper_policy_cache_hits_total`
- `reaper_active_policies`

### Grafana Dashboards (Future)

Pre-built dashboards for:
- Policy evaluation latency (p50, p95, p99)
- Throughput (requests/second)
- Cache hit rates
- Policy count and distribution

---

## 🔐 Security Hardening

### Production Recommendations

1. **Use secrets for sensitive data**:
```yaml
secrets:
  platform_key:
    file: ./secrets/platform.key
```

2. **Enable read-only root filesystem**:
```yaml
security_opt:
  - no-new-privileges:true
read_only: true
tmpfs:
  - /tmp
```

3. **Limit resources**:
```yaml
deploy:
  resources:
    limits:
      cpus: '2.0'
      memory: 1G
    reservations:
      cpus: '0.5'
      memory: 512M
```

4. **Network isolation**:
```yaml
networks:
  reaper-net:
    driver: bridge
    internal: true  # No external access
```

---

## 🚀 Next Steps

- [ ] Deploy to Kubernetes (see [Kubernetes Deployment Guide](./KUBERNETES_DEPLOYMENT.md))
- [ ] Set up monitoring (Prometheus + Grafana)
- [ ] Configure load balancing
- [ ] Implement high availability
- [ ] Set up policy CI/CD pipeline

---

## 📚 Related Documentation

- [Phase 8.1 Plan](./PHASE_8.1_PLAN.md) - Complete roadmap
- [Deployment Patterns](./DEPLOYMENT_PATTERNS.md) - Architecture patterns
- [Sidecar Deployment](./SIDECAR_DEPLOYMENT.md) - Kubernetes sidecar pattern

---

*Last updated: 2025-12-14*
*Phase 8.1: Docker Deployment*
