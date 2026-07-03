# UDS Deployment Models

The Reaper Agent can serve policy decisions over a **Unix Domain Socket (UDS)**
in addition to (or instead of) TCP. UDS bypasses the TCP/IP stack, so for a
co-located client (sidecar, same-host service) it delivers lower latency and
higher throughput than loopback TCP — typically **~15–20% higher throughput and
lower tail latency** for the same work.

There are two first-class UDS deployment models, selected by a single config
knob (`shards`).

## The two models

### Shared (default)

One socket, served by the agent's shared multi-threaded runtime.

```
client ──▶ /run/reaper/agent.sock ──▶ [ multi-threaded runtime, all cores ]
```

- Work-stealing across all cores → **best tail latency (p99)**.
- Simplest to operate: one socket, one mount.
- **Recommended default.** Pick this unless you have measured a throughput
  ceiling and confirmed the agent is CPU-bound.

### Sharded / thread-per-core

N sockets, each served by its own single-thread runtime pinned to a core
(share-nothing).

```
client ──▶ /run/reaper/agent-0.sock ──▶ [ runtime pinned to core 0 ]
       ──▶ /run/reaper/agent-1.sock ──▶ [ runtime pinned to core 1 ]
       ──▶ /run/reaper/agent-2.sock ──▶ [ runtime pinned to core 2 ]
       ──▶ /run/reaper/agent-3.sock ──▶ [ runtime pinned to core 3 ]
```

- Share-nothing (no cross-core cache bouncing, no work-stealing, pinned cores)
  → **~12–17% higher throughput and ~30% lower median latency** under
  saturation.
- **Tradeoff: worse p99.** With fixed shards and round-robin connection
  assignment, an unlucky connection can't be rebalanced across cores — the
  classic thread-per-core tail-latency cost.
- UDS has no `SO_REUSEPORT`, so **multiple socket files** is how a
  thread-per-core UDS server is sharded. **Clients must round-robin their
  connections** across `agent-0.sock … agent-{N-1}.sock`.
- Best when the agent has dedicated cores and you are throughput-bound at
  saturation (e.g. a busy sidecar fronting a hot service).

See [`docs/performance/THROUGHPUT_HARNESS.md`](../performance/THROUGHPUT_HARNESS.md)
for the measured numbers and the reproducible benchmark
(`services/reaper-agent/examples/uds_shard.rs`).

## Configuration

| Setting | Env var | Default | Meaning |
|---------|---------|---------|---------|
| `uds.enabled` | `REAPER_UDS_ENABLED` | `false` | Enable the UDS listener(s) |
| `uds.socket_path` | `REAPER_UDS_PATH` | `/var/run/reaper/agent.sock` | Socket path (base path in sharded mode) |
| `uds.socket_permissions` | `REAPER_UDS_PERMISSIONS` | `0660` (octal) | Socket file mode |
| `uds.shards` | `REAPER_UDS_SHARDS` | `0` | `0`/`1` = shared; `N>1` = sharded thread-per-core |
| `uds.pin_cores` | `REAPER_UDS_PIN_CORES` | `true` | Pin each shard runtime to a core (sharded only) |

In sharded mode the shard index is inserted before the file extension:
`/run/reaper/agent.sock` → `agent-0.sock`, `agent-1.sock`, …

### Examples

Shared (default), just enable it:

```bash
REAPER_UDS_ENABLED=true
REAPER_UDS_PATH=/run/reaper/agent.sock
```

Sharded, one shard per core, pinned:

```bash
REAPER_UDS_ENABLED=true
REAPER_UDS_PATH=/run/reaper/agent.sock
REAPER_UDS_SHARDS=4
REAPER_UDS_PIN_CORES=true
```

## Security — filesystem permissions ARE the boundary

UDS has **no application-layer authentication**. The access-control boundary is
purely filesystem permissions, so the agent enforces them for you:

1. The socket's **parent directory is created owner-only (`0700`)**. No other
   user can traverse into it to reach the socket — even during the brief window
   between `bind()` and the chmod, and even if the socket's own mode is loose.
2. **Every socket is chmod'd** to `socket_permissions` (default `0o660`:
   owner + group read/write).
3. If `socket_permissions` grants access to *other* (`0o007` bits set), the
   agent **logs a warning at startup** — that would let any user on the host
   call the agent.

In **sharded mode all N sockets live in that one `0700` directory**, so a single
directory boundary secures every mount; each socket is still chmod'd
individually.

**Client access model:** run the client as the **same user** as the agent, or
in the **same group** with `socket_permissions = 0o660`. Never use `0o666`.

## Packaging the mounts securely

### systemd

Use `RuntimeDirectory=` — systemd creates `/run/reaper` owned by the service
user, mode `0700`, and cleans it up on stop:

```ini
[Service]
User=reaper
Group=reaper
RuntimeDirectory=reaper
RuntimeDirectoryMode=0700
Environment=REAPER_UDS_ENABLED=true
Environment=REAPER_UDS_PATH=/run/reaper/agent.sock
Environment=REAPER_UDS_SHARDS=4
ExecStart=/usr/bin/reaper-agent
```

A client that needs access joins the `reaper` group and uses
`REAPER_UDS_PERMISSIONS=0660`.

### Kubernetes / Docker (sidecar)

Share an **`emptyDir`** between the agent and its client sidecar; the socket(s)
live on that shared volume. Run both containers as the same `runAsUser` (or a
shared `fsGroup`) so the client can open the socket.

```yaml
spec:
  securityContext:
    runAsUser: 65532
    runAsNonRoot: true
  volumes:
    - name: reaper-uds
      emptyDir: {}                 # shared socket dir, pod-local
  containers:
    - name: reaper-agent
      env:
        - { name: REAPER_UDS_ENABLED, value: "true" }
        - { name: REAPER_UDS_PATH,    value: "/run/reaper/agent.sock" }
        - { name: REAPER_UDS_SHARDS,  value: "4" }   # sharded; omit/1 for shared
      volumeMounts:
        - { name: reaper-uds, mountPath: /run/reaper }
    - name: app                    # your service, the UDS client
      volumeMounts:
        - { name: reaper-uds, mountPath: /run/reaper, readOnly: false }
```

The `emptyDir` is pod-scoped, so the socket is never exposed outside the pod.
For sharded mode, the client connects round-robin across
`/run/reaper/agent-0.sock … agent-{N-1}.sock`.

## Choosing a model — quick guide

- **Start with shared.** It's simpler and has the best p99.
- **Move to sharded** only when: you've confirmed the agent is CPU-bound at your
  target load, it has dedicated cores, and you care more about peak throughput /
  median latency than about p99. Set `shards` = number of dedicated cores.
- **Client must round-robin** across the shard sockets in sharded mode, or all
  load lands on one core and you get the worst of both worlds.
