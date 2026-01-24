# Event-Driven Policy Loading

This document describes the event-driven architecture for policy synchronization between the Reaper Management Server and Agents.

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                    Management Server                              │
│  ┌─────────────┐    ┌──────────────┐    ┌────────────────┐      │
│  │  Bundle API │    │ Event Emitter │    │  SSE Endpoint  │      │
│  │             │───>│               │───>│ /orgs/{}/events│      │
│  └─────────────┘    └──────────────┘    └───────┬────────┘      │
└─────────────────────────────────────────────────┼───────────────┘
                                                  │ SSE Stream
                                                  ▼
┌─────────────────────────────────────────────────────────────────┐
│                         Agent Fleet                               │
│                                                                   │
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐          │
│  │   Agent 1   │    │   Agent 2   │    │   Agent N   │          │
│  │  (Region A) │    │  (Region B) │    │  (Region C) │          │
│  └──────┬──────┘    └──────┬──────┘    └──────┬──────┘          │
│         │ Deploy            │ Deploy           │ Deploy          │
│         ▼                   ▼                  ▼                 │
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐          │
│  │Policy Engine│    │Policy Engine│    │Policy Engine│          │
│  └─────────────┘    └─────────────┘    └─────────────┘          │
└─────────────────────────────────────────────────────────────────┘
```

## Event Types

### bundle.promoted

Emitted when a bundle is promoted to production.

```json
{
  "event_type": "bundle.promoted",
  "org_id": "uuid",
  "timestamp": "2024-01-15T10:30:00Z",
  "data": {
    "bundle_id": "uuid",
    "name": "rbac-policy-v2",
    "version": "2.0.0",
    "checksum": "sha256:abcd1234...",
    "promoted_by": "user@example.com"
  }
}
```

### bundle.deprecated

Emitted when a bundle is deprecated or replaced.

```json
{
  "event_type": "bundle.deprecated",
  "org_id": "uuid",
  "timestamp": "2024-01-15T10:30:00Z",
  "data": {
    "bundle_id": "uuid",
    "reason": "replaced_by_newer_version",
    "replacement_bundle_id": "uuid"
  }
}
```

### data.sync

Emitted when entity data changes (optional, for ABAC scenarios).

```json
{
  "event_type": "data.sync",
  "org_id": "uuid",
  "timestamp": "2024-01-15T10:30:00Z",
  "data": {
    "sync_type": "incremental",
    "affected_entities": 150,
    "source": "api_update"
  }
}
```

### agent.config_updated

Emitted when agent configuration changes.

```json
{
  "event_type": "agent.config_updated",
  "org_id": "uuid",
  "timestamp": "2024-01-15T10:30:00Z",
  "data": {
    "config_key": "cache_ttl",
    "old_value": "300",
    "new_value": "600"
  }
}
```

## Agent Configuration

### Enabling Managed Mode

Set the following environment variables to enable management connection:

```bash
# Enable management connection
REAPER_MANAGEMENT_ENABLED=true

# Management server URL
REAPER_MANAGEMENT_URL=http://management:3000

# Organization identifier
REAPER_MANAGEMENT_ORG=default

# API key for authentication
REAPER_MANAGEMENT_API_KEY=your-api-key-here
```

### Configuration Options

| Variable | Default | Description |
|----------|---------|-------------|
| `REAPER_MANAGEMENT_ENABLED` | `false` | Enable management plane connection |
| `REAPER_MANAGEMENT_URL` | - | Management server URL |
| `REAPER_MANAGEMENT_ORG` | - | Organization slug |
| `REAPER_MANAGEMENT_API_KEY` | - | API key for registration |
| `REAPER_MANAGEMENT_HEARTBEAT_INTERVAL` | `30` | Heartbeat interval (seconds) |
| `REAPER_MANAGEMENT_POLL_INTERVAL` | `60` | Bundle polling interval (seconds) |
| `REAPER_MANAGEMENT_SYNC_ON_STARTUP` | `true` | Sync bundle on startup |

## Sync Flow

### Registration Flow

1. Agent starts with management configuration
2. Agent calls `POST /orgs/{org}/agents/register` with API key
3. Management server creates/updates agent record
4. Returns JWT token and agent ID
5. Agent stores token for subsequent requests

### Bundle Sync Flow

1. Agent receives `bundle.promoted` event (or polls for updates)
2. Agent calls `GET /orgs/{org}/bundles/promoted` to get bundle info
3. Compares checksum with current bundle
4. If different, calls `GET /orgs/{org}/bundles/{id}/download`
5. Validates bundle checksum
6. Deploys bundle to policy engine (atomic hot-swap)
7. Reports success via heartbeat

### Heartbeat Flow

Periodic heartbeats (default: 30s) include:

- Agent status
- Current bundle ID and checksum
- Performance metrics
- Error counts

## Fallback Behavior

When SSE connection fails, agents fallback to polling:

1. SSE connection attempt fails
2. Agent logs warning and continues
3. Polling interval activated (default: 60s)
4. Agent periodically checks for bundle updates
5. SSE reconnection attempted on next heartbeat

## Error Handling

### Connection Errors

- Initial connection timeout: 10s
- Reconnection backoff: exponential (1s, 2s, 4s, 8s, max 60s)
- Max reconnection attempts: unlimited (with backoff)

### Bundle Deployment Errors

- Invalid bundle format: Reject, keep current
- Checksum mismatch: Reject, keep current
- Policy engine error: Log, retry on next poll

## Metrics

| Metric | Type | Description |
|--------|------|-------------|
| `reaper_management_events_published_total` | Counter | SSE events published |
| `reaper_management_sse_subscribers` | Gauge | Current SSE subscribers |
| `reaper_agent_bundle_syncs_total` | Counter | Bundle sync operations |
| `reaper_agent_bundle_sync_errors_total` | Counter | Bundle sync errors |

## Security Considerations

1. **API Key Protection**: Store API keys securely, use environment variables
2. **JWT Tokens**: Short-lived (1 hour), auto-refreshed
3. **TLS**: Always use HTTPS in production
4. **Network Isolation**: Use private networks between management and agents
5. **Bundle Validation**: Always verify checksums before deployment

## Troubleshooting

### Agent Not Receiving Events

1. Check `REAPER_MANAGEMENT_ENABLED=true`
2. Verify management URL is reachable
3. Check API key is valid
4. Look for connection errors in agent logs
5. Verify firewall allows SSE connections

### Bundle Not Deploying

1. Check agent logs for deployment errors
2. Verify bundle checksum matches
3. Check policy engine errors
4. Ensure sufficient memory for bundle

### High Latency Updates

1. Consider reducing poll interval
2. Check network latency to management
3. Monitor SSE connection health
4. Check management server load

## Example Docker Setup

```yaml
# Agent with management connection
agent:
  environment:
    - REAPER_MANAGEMENT_ENABLED=true
    - REAPER_MANAGEMENT_URL=http://management:3000
    - REAPER_MANAGEMENT_ORG=myorg
    - REAPER_MANAGEMENT_API_KEY=${API_KEY}
  depends_on:
    management:
      condition: service_healthy
```

## Related Documentation

- [Bundle Format](BUNDLE_FORMAT.md) - Bundle file specifications
- [OPERATIONS_GUIDE](../deployment/OPERATIONS_GUIDE.md) - Deployment operations
- [Architecture](../architecture/ARCHITECTURE.md) - System architecture
