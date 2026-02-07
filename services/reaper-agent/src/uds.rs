//! Unix Domain Socket (UDS) listener for the Reaper Agent.
//!
//! Provides a UDS transport alongside TCP for lower-latency same-host IPC.
//! UDS bypasses the TCP/IP stack, reducing latency by 20-40% for local calls.

use axum::Router;
use std::path::{Path, PathBuf};
use tokio::net::UnixListener;
use tracing::{error, info};

/// Serve the given axum router over a Unix Domain Socket.
///
/// This function:
/// 1. Removes any stale socket file from a previous run
/// 2. Creates the parent directory if needed
/// 3. Binds a `UnixListener` to the socket path
/// 4. Sets file permissions on the socket
/// 5. Serves the router via `axum::serve`
///
/// The socket file is cleaned up when the function returns (graceful shutdown).
pub async fn serve_uds(
    socket_path: PathBuf,
    permissions: u32,
    app: Router<()>,
) -> anyhow::Result<()> {
    // Remove stale socket file if it exists from a previous run
    if socket_path.exists() {
        info!(
            path = %socket_path.display(),
            "Removing stale UDS socket file"
        );
        std::fs::remove_file(&socket_path)?;
    }

    // Create parent directory if needed
    if let Some(parent) = socket_path.parent() {
        if !parent.exists() {
            info!(
                dir = %parent.display(),
                "Creating UDS socket directory"
            );
            std::fs::create_dir_all(parent)?;
        }
    }

    // Bind the Unix listener
    let uds_listener = UnixListener::bind(&socket_path)?;

    // Set socket file permissions
    set_socket_permissions(&socket_path, permissions)?;

    info!(
        path = %socket_path.display(),
        permissions = format!("{:o}", permissions),
        "UDS listener started"
    );

    // Serve using axum's native UnixListener support
    let result = axum::serve(uds_listener, app).await;

    // Cleanup socket file on exit
    cleanup_socket(&socket_path);

    result.map_err(|e| anyhow::anyhow!("UDS server error: {}", e))
}

/// Set file permissions on the socket file (Unix-only).
#[cfg(unix)]
fn set_socket_permissions(path: &Path, mode: u32) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let perms = std::fs::Permissions::from_mode(mode);
    std::fs::set_permissions(path, perms)?;
    Ok(())
}

#[cfg(not(unix))]
fn set_socket_permissions(_path: &Path, _mode: u32) -> anyhow::Result<()> {
    // No-op on non-Unix platforms
    Ok(())
}

/// Remove the socket file if it exists.
fn cleanup_socket(path: &Path) {
    if path.exists() {
        if let Err(e) = std::fs::remove_file(path) {
            error!(
                path = %path.display(),
                error = %e,
                "Failed to clean up UDS socket file"
            );
        } else {
            info!(path = %path.display(), "UDS socket file cleaned up");
        }
    }
}
