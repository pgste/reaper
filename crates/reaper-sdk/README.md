# Reaper SDK

High-performance Rust SDK for evaluating policies against the Reaper policy engine.

## Features

- **HTTP Client**: Simple RESTful client for policy evaluation (1-2ms latency)
- **Bundle Deployment**: Deploy policy bundles (.rbb format) with zero-downtime hot-reload
- **Connection Pooling**: Automatic connection reuse for high throughput
- **Type Safety**: Strongly-typed requests and responses

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
reaper-sdk = { path = "../path/to/reaper/crates/reaper-sdk" }
tokio = { version = "1.0", features = ["full"] }
```

## Quick Start

```rust
use reaper_sdk::{ReaperClient, PolicyRequest, Decision};
use std::collections::HashMap;

#[tokio::main]
async fn main() -> reaper_sdk::Result<()> {
    // Create HTTP client
    let client = ReaperClient::http("http://localhost:8080")?;

    // Check agent health
    client.health_check().await?;

    // Evaluate a policy
    let request = PolicyRequest {
        policy_id: "my-policy".to_string(),
        principal: "user:alice".to_string(),
        action: "read".to_string(),
        resource: "/api/data".to_string(),
        context: HashMap::new(),
    };

    let response = client.evaluate(request).await?;

    match response.decision {
        Decision::Allow => println!("ACCESS GRANTED"),
        Decision::Deny => println!("ACCESS DENIED"),
    }

    Ok(())
}
```

## Running the Example

1. Start the Reaper Agent:
   ```bash
   cargo run --bin reaper-agent
   ```

2. Start the Reaper Platform:
   ```bash
   cargo run --bin reaper-platform
   ```

3. Run the SDK example:
   ```bash
   cargo run --example basic_usage
   ```

## Bundle Deployment

Deploy a policy bundle with zero-downtime hot-reload:

```rust
let bundle_bytes = std::fs::read("policy.rbb")?;
let response = client.deploy_bundle(&bundle_bytes, "1.0.0", false).await?;

println!("Deployed policy: {}", response.policy_id);
println!("Version: {}", response.version);
```

## Performance

- **Policy Evaluation**: 1-2ms typical latency over HTTP
- **Connection Pooling**: Up to 10 idle connections per host
- **Timeout**: 5 second default timeout
- **Throughput**: >1000 requests/second per client

## Architecture

```
SDK Client  ──HTTP──>  Agent (8080)  ──>  PolicyEngine  ──>  eBPF (optional)
```

The SDK communicates with a Reaper Agent which evaluates policies using a lock-free
in-memory engine with sub-microsecond latency for simple policies.

## Future Features

- UDP protocol for extreme performance (50-200µs latency)
- Unix socket support for local deployments
- Entity CRUD operations
- Batch evaluation

## License

MIT OR Apache-2.0
