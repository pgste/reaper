# Reaper Management Server

Multi-tenant policy management server for the Reaper platform. Provides centralized policy storage, versioning, compilation, and deployment coordination.

## Features

- **Multi-Tenant Architecture**: Organizations as the primary tenancy unit with team-level subdivision
- **Policy Sources**: Sync policies from Git repositories or external HTTP APIs
- **Bundle Workflow**: Draft → Compiled → Staged → Promoted lifecycle
- **Pluggable Storage**: Filesystem, S3, MongoDB, DynamoDB backends
- **Authentication**: API keys and JWKS-based JWT validation
- **Real-time Events**: Server-Sent Events for agent notifications
- **Agent Management**: Self-registration and health monitoring

## Quick Start

```bash
# Run with default configuration
cargo run --release

# Run with custom config
cargo run --release -- --config /etc/reaper/management.yaml

# Run with environment overrides
REAPER_PORT=8081 REAPER_DATABASE_URL=sqlite:data/mgmt.db cargo run --release
```

## Configuration

### Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `REAPER_PORT` | `8081` | Server port |
| `REAPER_BIND_ADDRESS` | `0.0.0.0` | Bind address |
| `REAPER_DATABASE_TYPE` | `sqlite` | Database type (sqlite, postgres) |
| `REAPER_DATABASE_URL` | `sqlite:data/reaper.db` | Database connection URL |
| `REAPER_STORAGE_TYPE` | `filesystem` | Storage backend |
| `REAPER_STORAGE_PATH` | `data/bundles` | Filesystem storage path |

### Configuration File (YAML)

```yaml
server:
  port: 8081
  bind_address: "0.0.0.0"

database:
  db_type: sqlite
  url: "sqlite:data/reaper.db"
  max_connections: 10

storage:
  storage_type: filesystem
  filesystem:
    path: "data/bundles"
  # For S3:
  # s3:
  #   bucket: my-bucket
  #   region: us-east-1
  #   prefix: bundles/
```

## API Reference

### Organizations

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/v1/orgs` | List organizations |
| POST | `/api/v1/orgs` | Create organization |
| GET | `/api/v1/orgs/{org}` | Get organization |
| PUT | `/api/v1/orgs/{org}` | Update organization |
| DELETE | `/api/v1/orgs/{org}` | Delete organization |

### Policies

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/v1/orgs/{org}/policies` | List policies |
| POST | `/api/v1/orgs/{org}/policies` | Create policy |
| GET | `/api/v1/orgs/{org}/policies/{id}` | Get policy |
| PUT | `/api/v1/orgs/{org}/policies/{id}` | Update policy |
| DELETE | `/api/v1/orgs/{org}/policies/{id}` | Delete policy |
| GET | `/api/v1/orgs/{org}/policies/{id}/versions` | List versions |

### Policy Sources

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/v1/orgs/{org}/sources` | List sources |
| POST | `/api/v1/orgs/{org}/sources` | Create source |
| GET | `/api/v1/orgs/{org}/sources/{id}` | Get source |
| PUT | `/api/v1/orgs/{org}/sources/{id}` | Update source |
| DELETE | `/api/v1/orgs/{org}/sources/{id}` | Delete source |
| POST | `/api/v1/orgs/{org}/sources/{id}/sync` | Trigger sync |

### Bundles

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/v1/orgs/{org}/bundles` | List bundles |
| POST | `/api/v1/orgs/{org}/bundles` | Create bundle |
| GET | `/api/v1/orgs/{org}/bundles/{id}` | Get bundle |
| PUT | `/api/v1/orgs/{org}/bundles/{id}` | Update bundle |
| DELETE | `/api/v1/orgs/{org}/bundles/{id}` | Delete bundle |
| POST | `/api/v1/orgs/{org}/bundles/{id}/policies` | Add policies |
| DELETE | `/api/v1/orgs/{org}/bundles/{id}/policies` | Remove policies |
| POST | `/api/v1/orgs/{org}/bundles/{id}/compile` | Compile bundle |
| POST | `/api/v1/orgs/{org}/bundles/{id}/stage` | Stage bundle |
| POST | `/api/v1/orgs/{org}/bundles/{id}/promote` | Promote bundle |
| POST | `/api/v1/orgs/{org}/bundles/{id}/deprecate` | Deprecate bundle |
| GET | `/api/v1/orgs/{org}/bundles/{id}/download` | Download compiled bundle |
| GET | `/api/v1/orgs/{org}/bundles/promoted` | Get promoted bundle |

### Agents

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/v1/orgs/{org}/agents` | List agents |
| POST | `/api/v1/orgs/{org}/agents/register` | Register agent |
| GET | `/api/v1/orgs/{org}/agents/{id}` | Get agent |
| POST | `/api/v1/orgs/{org}/agents/{id}/heartbeat` | Agent heartbeat |
| DELETE | `/api/v1/orgs/{org}/agents/{id}` | Deregister agent |

### Events (SSE)

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/v1/orgs/{org}/events` | SSE event stream |

### Authentication

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/api/v1/orgs/{org}/auth/api-keys` | Create API key |
| GET | `/api/v1/orgs/{org}/auth/api-keys` | List API keys |
| DELETE | `/api/v1/orgs/{org}/auth/api-keys/{id}` | Revoke API key |

### Health

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/health` | Health check |
| GET | `/health/ready` | Readiness probe |

## Bundle Workflow

```
┌─────────┐    compile    ┌──────────┐    stage    ┌────────┐    promote    ┌──────────┐
│  Draft  │ ────────────> │ Compiled │ ──────────> │ Staged │ ───────────> │ Promoted │
└─────────┘               └──────────┘             └────────┘              └──────────┘
     │                          │                       │                        │
     │                          │                       │                        │
     └──────────────────────────┴───────────────────────┴────────────────────────┘
                                        │
                                   deprecate
                                        │
                                        v
                                  ┌────────────┐
                                  │ Deprecated │
                                  └────────────┘
```

1. **Draft**: Initial state, add/remove policies
2. **Compiled**: Bundle compiled to binary format (.rbb)
3. **Staged**: Ready for testing/validation
4. **Promoted**: Live in production, agents notified
5. **Deprecated**: Archived, replaced by newer bundle

## Policy Source Types

### Git Repository

```json
{
  "name": "main-policies",
  "source_type": "git",
  "config": {
    "url": "https://github.com/org/policies.git",
    "branch": "main",
    "path": "policies/",
    "patterns": ["*.reap", "*.yaml"],
    "auth": {
      "type": "token",
      "token": "ghp_xxx"
    }
  },
  "sync_interval_secs": 300
}
```

### External API

```json
{
  "name": "policy-api",
  "source_type": "api",
  "config": {
    "url": "https://api.example.com/policies",
    "method": "GET",
    "headers": {
      "Accept": "application/json"
    },
    "api_key_header": "X-API-Key",
    "api_key": "secret",
    "format": "json",
    "jsonpath": "$.policies[*]"
  },
  "sync_interval_secs": 600
}
```

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                    Reaper Management Server                      │
├─────────────────────────────────────────────────────────────────┤
│  API Layer (Axum)                                               │
│  ├── Organizations, Teams                                       │
│  ├── Policies, Versions                                         │
│  ├── Sources (Git, API)                                         │
│  ├── Bundles (Compile, Promote)                                 │
│  ├── Agents (Register, Heartbeat)                               │
│  └── Events (SSE)                                               │
├─────────────────────────────────────────────────────────────────┤
│  Service Layer                                                  │
│  ├── BundleService (compile, stage, promote)                    │
│  ├── SyncService (git clone/pull, API fetch)                    │
│  └── AuthService (API keys, JWKS)                               │
├─────────────────────────────────────────────────────────────────┤
│  Repository Layer                                               │
│  ├── OrganizationRepository                                     │
│  ├── PolicyRepository                                           │
│  ├── BundleRepository                                           │
│  ├── SourceRepository                                           │
│  └── AgentRepository                                            │
├─────────────────────────────────────────────────────────────────┤
│  Storage Layer                                                  │
│  ├── FilesystemStorage                                          │
│  ├── S3Storage (feature: storage-s3)                            │
│  ├── MongoDbStorage (feature: storage-mongodb)                  │
│  └── DynamoDbStorage (feature: storage-dynamodb)                │
├─────────────────────────────────────────────────────────────────┤
│  Database Layer (SQLx)                                          │
│  ├── SQLite (default)                                           │
│  └── PostgreSQL                                                 │
└─────────────────────────────────────────────────────────────────┘
```

## Development

```bash
# Run tests
cargo test

# Run with logging
RUST_LOG=reaper_management=debug cargo run

# Build release
cargo build --release

# Build with S3 support
cargo build --release --features storage-s3
```

## License

Apache 2.0
