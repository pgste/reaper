# Docker Deployment for Reaper vs OPA Benchmarks

## Quick Start

Start both Reaper and OPA servers:

```bash
docker-compose up -d
```

Wait for services to be healthy:

```bash
docker-compose ps
```

Deploy policies and run benchmarks:

```bash
# Deploy ABAC policy to both engines
./bin/deploy-reaper.sh abac 10k
./bin/deploy-opa.sh abac 10k

# Run benchmark
./bin/benchmark.sh --scenario abac --scale 10k --requests 1000
```

## Services

- **Reaper Agent**: http://localhost:8080
  - Health: http://localhost:8080/health
  - Metrics: http://localhost:8080/metrics

- **OPA Server** (Styra Enterprise OPA - open source): http://localhost:8181
  - Health: http://localhost:8181/health
  - Policies: http://localhost:8181/v1/policies
  - Note: Uses Styra's Enterprise OPA with performance optimizations

## Environment Variables

### Reaper Agent

- `RUST_LOG`: Log level (default: `info`)
- `REAPER_LOG_FORMAT`: Log format - `json` or `pretty` (default: `json`)
- `OTEL_ENABLED`: Enable OpenTelemetry tracing - `true` or `false` (default: `false`)
- `OTEL_ENDPOINT`: OTLP endpoint (required when `OTEL_ENABLED=true`)

**Default behavior** (OTEL disabled):
```bash
# Logs only, no distributed tracing
docker-compose up -d
```

**With OpenTelemetry enabled**:
```bash
# Modify docker-compose.yml environment:
services:
  reaper:
    environment:
      - RUST_LOG=info
      - OTEL_ENABLED=true
      - OTEL_ENDPOINT=http://jaeger:4317

# Or override at runtime:
docker-compose up -d
docker-compose exec reaper sh -c 'OTEL_ENABLED=true OTEL_ENDPOINT=http://jaeger:4317 reaper-agent'
```

**Note**: When `OTEL_ENABLED=true`, you must provide `OTEL_ENDPOINT` or the agent will fail to start.

## Building

Build Reaper image:

```bash
docker-compose build reaper
```

## Development

View logs:

```bash
# All services
docker-compose logs -f

# Reaper only
docker-compose logs -f reaper

# OPA only
docker-compose logs -f opa
```

Stop services:

```bash
docker-compose down
```

Clean up everything (including volumes):

```bash
docker-compose down -v
docker system prune -f
```

## Performance Notes

**Docker Overhead**: Docker adds ~50-100µs of latency compared to native execution due to network stack overhead. For the most accurate benchmarks, run natively. Docker is provided for:

- Easy deployment
- Consistent environments
- Testing integrations
- Production-like setups

**Enterprise OPA**: The benchmark uses Styra's Enterprise OPA (recently open-sourced), which includes performance optimizations over the standard OPA distribution. This provides a more competitive comparison against Reaper's optimizations.

## Troubleshooting

### Services not starting

Check logs:
```bash
docker-compose logs
```

### Port conflicts

If ports 8080 or 8181 are already in use, modify `docker-compose.yml`:

```yaml
ports:
  - "8090:8080"  # Map to different host port
```

### Memory limits

Add resource limits in `docker-compose.yml`:

```yaml
services:
  reaper:
    deploy:
      resources:
        limits:
          memory: 512M
```
