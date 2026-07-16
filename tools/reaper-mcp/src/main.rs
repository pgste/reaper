//! reaper-mcp — stdio MCP server gating tool calls through a Reaper Agent.
//!
//! stdout carries the protocol; all diagnostics go to stderr.

use anyhow::Context;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use reaper_mcp::{AdapterConfig, McpServer};

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let config = AdapterConfig::from_env().context("loading adapter configuration")?;
    tracing::info!(transport = ?config.transport, "reaper-mcp starting");
    let server = McpServer::new(config)?;

    let stdin = BufReader::new(tokio::io::stdin());
    let mut stdout = tokio::io::stdout();
    let mut lines = stdin.lines();

    while let Some(line) = lines.next_line().await.context("reading stdin")? {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(response) = server.handle_message(trimmed).await {
            stdout.write_all(response.as_bytes()).await?;
            stdout.write_all(b"\n").await?;
            stdout.flush().await?;
        }
    }
    tracing::info!("stdin closed, shutting down");
    Ok(())
}
