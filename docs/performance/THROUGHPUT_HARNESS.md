# Throughput Harness — HTTP vs UDS, Compiled vs AST

A self-contained, rerunnable load-test harness that measures the Reaper agent's
end-to-end request throughput and latency across two transports (loopback **TCP**
vs **Unix domain socket**) and two evaluator modes (**compiled** Reaper-DSL vs the
**AST** interpreter fallback).

Source: [`services/reaper-agent/examples/throughput.rs`](../../services/reaper-agent/examples/throughput.rs)

## What it does

It brings up the **real** agent router in-process — the same `evaluate_policy`
(serde_json) and `fast_evaluate_policy` (sonic-rs SIMD) handlers used in
production — and serves it **simultaneously over TCP and a UDS**, sharing one
`AgentState`. The same ABAC policy is deployed twice: once as the compiled
evaluator and once as the AST interpreter. A closed-loop load generator then
drives both transports using a single hyper HTTP/1.1 client stack with
keep-alive.

Because only the transport differs between the TCP and UDS runs (identical HTTP
parsing/serialization, identical handler, identical connection reuse), the
TCP-vs-UDS delta isolates the **round-trip cost of the network stack**. Likewise,
compiled-vs-AST isolates the **evaluator**, and fast-vs-standard isolates the
**JSON parsing** cost on the request path.

The benchmark matrix:

| transport | endpoint (parser) | evaluator |
|-----------|-------------------|-----------|
| TCP / UDS | `fast-messages` (sonic-rs) | compiled |
| TCP / UDS | `fast-messages` (sonic-rs) | AST |
| TCP / UDS | `messages` (serde_json)   | compiled |

Decision caching is **disabled** for the run so the numbers reflect actual
evaluation, not cache hits.

## Running

Always use the release profile — it enables fat LTO and the mimalloc allocator,
so the numbers are representative:

```bash
cargo run --release --example throughput -p reaper-agent
```

Tune the load:

```bash
cargo run --release --example throughput -p reaper-agent -- \
    --connections 16 \
    --duration-secs 5 \
    --warmup-secs 1
```

- `--connections` — number of concurrent keep-alive connections (closed-loop
  workers). This is the offered concurrency.
- `--duration-secs` — measurement window per configuration.
- `--warmup-secs` — discarded warmup window per configuration.

## Output

Each row reports throughput (`req/s`) and the latency distribution
(`p50/p95/p99/max`, in microseconds), followed by derived ratios:

- **UDS vs TCP** (fast, compiled) — the transport speedup.
- **compiled vs AST** — the evaluator speedup at the full request level.
- **fast(sonic) vs std(serde)** — the JSON-parser speedup on the request path.

## Example output

From a 4-core sandbox over loopback (release build, 16 connections, 3s measure).
Numbers are environment-specific — rerun on your target hardware:

```
config                        req/s    p50(µs)    p95(µs)    p99(µs)    max(µs)
------------------------------------------------------------------------------
TCP  fast  compiled           72080     212.33     343.94     434.81    4783.60
UDS  fast  compiled           84376     174.77     308.91     419.35    6243.15
TCP  fast  ast                71251     217.49     337.57     403.81    4629.45
UDS  fast  ast                79719     190.73     311.44     388.55    3824.29
TCP  std   compiled           64222     236.81     394.39     494.98    6366.97
UDS  std   compiled           83102     181.98     302.08     388.47    4083.43

  UDS vs TCP  (fast, compiled):  1.17x
  compiled vs AST (TCP, fast):   1.01x
  compiled vs AST (UDS, fast):   1.06x
  fast(sonic) vs std(serde) TCP: 1.12x
```

Takeaways from this run:
- **UDS is ~17% higher throughput and noticeably lower p50** than TCP for the
  same work — the transport is a real win for co-located sidecars.
- **compiled vs AST is only ~1–6% at the request level**, even though the pure
  evaluator is 1.9–3.8× faster (`complex_policy_bench`). That is expected: a
  request spends most of its time in the socket round trip and JSON parsing, so
  the (sub-microsecond) evaluator is a small slice. Compiled still wins, and the
  gap grows with policy complexity and shrinks with transport/parse overhead.
- **sonic-rs (fast) beats serde_json (std) by ~12%** on TCP — JSON parsing is a
  meaningful fraction of a small authz request.

## Scaling up (can we hit 100k/s?)

Same 4-core sandbox at `--connections 48` (server *and* load generator share the
4 cores, so this is a pessimistic in-process ceiling):

```
config                        req/s    p50(µs)    p99(µs)
TCP  fast  compiled           77825     605.51    1094.70
UDS  fast  compiled           85114     554.89     971.48
TCP  fast  ast                64150     737.08    1288.53
UDS  fast  ast                74623     634.33    1103.03

  UDS vs TCP  (fast, compiled):  1.09x
  compiled vs AST (TCP, fast):   1.21x   <- gap WIDENS under load
  compiled vs AST (UDS, fast):   1.14x
```

Two things to note:

- **Throughput plateaus (~85k req/s) because the harness shares 4 cores between
  the server and the load generator.** The evaluator is nowhere near the limit:
  a compiled ABAC eval is ~330–780 ns, i.e. **1.3–3M evals/sec per core**. So
  **100k+ req/s is comfortably achievable** once the agent has dedicated cores
  (real deployment) — the bottleneck is the request round-trip and CPU sharing,
  not evaluation, which has ~50–100× headroom over 100k/s.
- **The compiled advantage GROWS under saturation** — from ~1–6% at low
  concurrency to **14–21%** here. When the box is CPU-bound, doing less work per
  evaluation converts directly into more requests served. This is the strongest
  argument for compiled mode: it's not just lower latency, it's more throughput
  per core exactly when you're capacity-constrained.

## Interpreting the results

- The **eval itself is sub-microsecond** (see `cargo bench -p policy-engine
  --bench complex_policy_bench`), so at the request level most of the time is
  transport + HTTP framing + JSON parsing. Expect the compiled-vs-AST gap to be
  **smaller here than in the pure-evaluator benchmark** — the evaluator is a
  small slice of a request that also pays for the socket round trip and parsing.
- **UDS removes the TCP/IP stack** (no loopback IP routing, no TCP
  handshake/teardown across the pool, smaller framing overhead), so it typically
  shows higher throughput and lower tail latency than TCP for the same work.
  This is a transport-level win and should be reported separately from any
  engine comparison.
- **fast vs standard** shows why the SIMD path matters: serde_json fully
  deserializes into owned structures, which on small authz payloads can cost
  more than the evaluation.

## Per-core sharding: shared runtime vs thread-per-core UDS

[`services/reaper-agent/examples/uds_shard.rs`](../../services/reaper-agent/examples/uds_shard.rs)
compares two server models with the load generator in a **separate process** (so
it never steals the server's cores):

- **shared** — one multi-threaded tokio runtime on a single UDS socket.
- **sharded** — N single-thread runtimes, each pinned to a core and owning its
  own UDS socket (`agent-0.sock … agent-{N-1}.sock`); the client round-robins
  connections across them. Since UDS has no `SO_REUSEPORT`, multiple socket files
  is how you shard a thread-per-core UDS server.

```bash
# server (terminal 1)
cargo run --release --example uds_shard -p reaper-agent -- server --mode sharded --shards 4 --dir /tmp/reaper-shard
# load (terminal 2)
cargo run --release --example uds_shard -p reaper-agent -- load --dir /tmp/reaper-shard --shards 4 --connections 64 --duration-secs 4
```

Results (4-core sandbox, generic build, compiled policy, UDS):

```
model                                    32 conn                64 conn
shared  (1 socket, multi-thread)     99,708 req/s p50 289µs   107,965 req/s p50 539µs
sharded (4 sockets, pinned)         111,599 req/s p50 207µs   125,836 req/s p50 383µs
```

- **Sharded is +12–17% throughput and ~30% lower p50** — share-nothing (no
  work-stealing, no cross-core cache bouncing, pinned cores) is a real win, and
  it comfortably clears **100k+ req/s on 4 cores** (generic build; `target-cpu`
  would push it higher).
- **Tradeoff: worse p99** (e.g. 1428µs vs 806µs at 32 conn). With fixed shards
  and round-robin connection assignment, an unlucky connection can't be
  rebalanced across cores. That is the classic thread-per-core tail-latency cost.

Security: the harness creates the socket directory owner-only (`0700`) and
chmods every socket `0600` — the same model as the agent's `serve_uds`, applied
to each of the N mounts (more mounts = more filesystem boundaries to secure, and
UDS has no application-layer auth).

## Notes / caveats

- Closed-loop (each worker waits for its response before sending the next), so
  `req/s ≈ connections / mean_latency`. Increase `--connections` to find the
  saturation point.
- Loopback only. Real-network numbers will differ (RTT dominates), but the
  TCP-vs-UDS and compiled-vs-AST *ratios* remain informative for co-located
  sidecar deployments — which is exactly where UDS is used.
- This harness compares Reaper against itself across transports/evaluators. For
  a Reaper-vs-OPA comparison, see `benchmarks/reaper-vs-opa/` (requires the OPA
  binary) and the parity-gated `services/reaper-bench/`.
