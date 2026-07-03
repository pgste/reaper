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
