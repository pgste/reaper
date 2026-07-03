//! Unix Domain Socket (UDS) listeners for the Reaper Agent.
//!
//! Provides a UDS transport alongside TCP for lower-latency same-host IPC.
//! UDS bypasses the TCP/IP stack, reducing latency by 20-40% for local calls.
//!
//! Two first-class deployment models, selected by [`UdsSettings::shards`]:
//!
//! - **Shared** (`shards <= 1`): one socket served by the agent's shared
//!   multi-threaded runtime. Work-stealing across all cores, best tail latency.
//! - **Sharded / thread-per-core** (`shards > 1`): N sockets, each served by a
//!   single-thread runtime pinned to a core (share-nothing). Higher throughput
//!   and lower median latency under saturation; slightly worse p99. UDS has no
//!   `SO_REUSEPORT`, so multiple socket files is how a thread-per-core UDS
//!   server shards; clients round-robin connections across them.
//!
//! ## Security
//!
//! UDS has **no application-layer auth** — filesystem permissions ARE the
//! access-control boundary. Both models: create the socket's parent directory
//! owner-only (`0700`), so no other user can reach the socket during the brief
//! bind→chmod window (or ever), and chmod each socket to the configured mode
//! (default `0o660`). In sharded mode all N sockets share that one `0700`
//! directory, so a single directory boundary secures every mount.

use axum::Router;
use reaper_core::config::UdsSettings;
use std::path::{Path, PathBuf};
use tokio::net::UnixListener;
use tracing::{error, info, warn};

/// Spawn UDS listener(s) according to the configured deployment model.
///
/// - Shared model (`shards <= 1`): spawns one listener on the current async
///   runtime (non-blocking; returns immediately).
/// - Sharded model (`shards > 1`): spawns N dedicated OS threads, each running
///   its own single-thread runtime (optionally pinned to a core) serving its
///   own socket. Threads are detached; they run until the process exits.
///
/// `app` is cloned per listener (axum `Router` is cheaply clonable; the agent
/// state inside is shared via `Arc`).
pub fn spawn_uds_listeners(settings: &UdsSettings, app: Router<()>) {
    // Warn once (not per shard) if the socket mode is world-accessible.
    if settings.socket_permissions & 0o007 != 0 {
        warn!(
            permissions = format!("{:o}", settings.socket_permissions),
            "UDS socket_permissions grant access to 'other' users; \
             anyone on the host could call the agent. Use 0o660/0o600."
        );
    }

    if settings.is_sharded() {
        spawn_sharded(settings, app);
    } else {
        let path = settings.socket_path.clone();
        let perms = settings.socket_permissions;
        info!(path = %path.display(), "Starting UDS listener (shared model)");
        tokio::spawn(async move {
            if let Err(e) = serve_uds(path, perms, app).await {
                error!("UDS server error: {}", e);
            }
        });
    }
}

/// Sharded / thread-per-core model: one pinned single-thread runtime + socket
/// per shard.
fn spawn_sharded(settings: &UdsSettings, app: Router<()>) {
    let n = settings.effective_shards();

    // Create the shared parent directory owner-only ONCE, before any bind, so
    // there is never a window where a shard socket is reachable by other users.
    if let Some(parent) = settings.socket_path.parent() {
        if !parent.exists() {
            if let Err(e) = create_dir_private(parent) {
                error!(dir = %parent.display(), error = %e, "Failed to create UDS socket directory");
                return;
            }
            info!(dir = %parent.display(), "Created UDS socket directory (0700)");
        }
    }

    let cores = if settings.pin_cores {
        core_affinity::get_core_ids().unwrap_or_default()
    } else {
        Vec::new()
    };

    info!(
        shards = n,
        pin_cores = settings.pin_cores,
        available_cores = cores.len(),
        "Starting UDS listeners (sharded thread-per-core model)"
    );

    for i in 0..n {
        let path = settings.shard_socket_path(i);
        let perms = settings.socket_permissions;
        let app = app.clone();
        // Round-robin cores; None if pinning disabled or no cores reported.
        let core = if cores.is_empty() {
            None
        } else {
            cores.get(i % cores.len()).copied()
        };

        let spawned = std::thread::Builder::new()
            .name(format!("uds-shard-{i}"))
            .spawn(move || {
                if let Some(core) = core {
                    core_affinity::set_for_current(core);
                }
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        error!(shard = i, error = %e, "Failed to build shard runtime");
                        return;
                    }
                };
                rt.block_on(async move {
                    match bind_uds(&path, perms) {
                        Ok(listener) => {
                            info!(shard = i, path = %path.display(), "UDS shard listener started");
                            if let Err(e) = axum::serve(listener, app).await {
                                error!(shard = i, error = %e, "UDS shard server error");
                            }
                            cleanup_socket(&path);
                        }
                        Err(e) => {
                            error!(shard = i, path = %path.display(), error = %e, "Failed to bind UDS shard socket");
                        }
                    }
                });
            });

        if let Err(e) = spawned {
            error!(shard = i, error = %e, "Failed to spawn UDS shard thread");
        }
    }
}

/// Serve the given axum router over a single Unix Domain Socket (shared model).
///
/// The socket file is cleaned up when the function returns (graceful shutdown).
pub async fn serve_uds(
    socket_path: PathBuf,
    permissions: u32,
    app: Router<()>,
) -> anyhow::Result<()> {
    let uds_listener = bind_uds(&socket_path, permissions)?;

    info!(
        path = %socket_path.display(),
        permissions = format!("{:o}", permissions),
        "UDS listener started"
    );

    let result = axum::serve(uds_listener, app).await;

    // Cleanup socket file on exit
    cleanup_socket(&socket_path);

    result.map_err(|e| anyhow::anyhow!("UDS server error: {}", e))
}

/// Prepare and bind a secured Unix socket:
/// 1. Remove any stale socket file from a previous run.
/// 2. Create the parent directory owner-only (`0700`) if missing — closes the
///    bind→chmod window and makes the socket unreachable by other users.
/// 3. Bind the `UnixListener`.
/// 4. Chmod the socket to `permissions`.
fn bind_uds(socket_path: &Path, permissions: u32) -> anyhow::Result<UnixListener> {
    // Remove stale socket file if it exists from a previous run
    if socket_path.exists() {
        info!(path = %socket_path.display(), "Removing stale UDS socket file");
        std::fs::remove_file(socket_path)?;
    }

    // Create parent directory owner-only (0700) so the socket is unreachable by
    // other users regardless of the socket file's own mode.
    if let Some(parent) = socket_path.parent() {
        if !parent.exists() {
            info!(dir = %parent.display(), "Creating UDS socket directory (0700)");
            create_dir_private(parent)?;
        }
    }

    let listener = UnixListener::bind(socket_path)?;
    set_socket_permissions(socket_path, permissions)?;
    Ok(listener)
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

/// Create a directory (and parents) with owner-only (0700) permissions on Unix.
#[cfg(unix)]
fn create_dir_private(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)?;
    Ok(())
}

#[cfg(not(unix))]
fn create_dir_private(path: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(path)?;
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

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use axum::routing::get;
    use std::os::unix::fs::PermissionsExt;
    use std::time::Duration;

    fn mode(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    async fn wait_for_socket(path: &Path) {
        for _ in 0..80 {
            if path.exists() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("socket {} never appeared", path.display());
    }

    /// The sharded model must create N secured sockets in a `0700` directory,
    /// each chmod'd to the configured mode, and each must actually serve.
    #[tokio::test]
    async fn sharded_creates_n_secured_sockets_and_serves() {
        let tmp = tempfile::tempdir().unwrap();
        // Point at a NOT-yet-existing subdirectory so the agent's own
        // owner-only (0700) directory-creation path runs (it deliberately does
        // not re-chmod an operator-provisioned directory that already exists).
        let sock_dir = tmp.path().join("run");
        let base = sock_dir.join("agent.sock");
        let settings = UdsSettings {
            enabled: true,
            socket_path: base.clone(),
            socket_permissions: 0o600,
            shards: 3,
            // Don't pin in tests — CI may restrict affinity.
            pin_cores: false,
        };

        let app: Router<()> = Router::new().route("/health", get(|| async { "ok" }));
        spawn_uds_listeners(&settings, app);

        // All three shard sockets must appear.
        for i in 0..3 {
            wait_for_socket(&settings.shard_socket_path(i)).await;
        }

        // Parent directory is owner-only; each socket carries the configured mode.
        assert_eq!(mode(&sock_dir), 0o700, "socket dir must be 0700");
        for i in 0..3 {
            let p = settings.shard_socket_path(i);
            assert!(p.exists(), "shard {i} socket missing");
            assert_eq!(mode(&p), 0o600, "shard {i} socket must be 0600");
        }

        // Each shard actually serves: a UDS connection + request returns 200.
        for i in 0..3 {
            let p = settings.shard_socket_path(i);
            let stream = tokio::net::UnixStream::connect(&p).await.unwrap();
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let (mut r, mut w) = stream.into_split();
            w.write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
            let mut buf = Vec::new();
            r.read_to_end(&mut buf).await.unwrap();
            let resp = String::from_utf8_lossy(&buf);
            assert!(
                resp.contains("200 OK"),
                "shard {i} did not serve 200: {resp}"
            );
        }
    }

    /// The shared model must create exactly one socket at the configured path.
    #[tokio::test]
    async fn shared_creates_single_socket() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.sock");
        let settings = UdsSettings {
            enabled: true,
            socket_path: path.clone(),
            socket_permissions: 0o600,
            shards: 0,
            pin_cores: false,
        };

        let app: Router<()> = Router::new().route("/health", get(|| async { "ok" }));
        spawn_uds_listeners(&settings, app);

        wait_for_socket(&path).await;
        assert_eq!(mode(&path), 0o600);
        // No shard-suffixed sockets in shared mode.
        assert!(!settings.shard_socket_path(0).exists());
    }
}
