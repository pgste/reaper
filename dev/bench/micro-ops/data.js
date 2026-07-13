window.BENCHMARK_DATA = {
  "lastUpdate": 1783956086711,
  "repoUrl": "https://github.com/pgste/reaper",
  "entries": {
    "Engine micro-ops (criterion)": [
      {
        "commit": {
          "author": {
            "email": "hwhbygwarm@gmail.com",
            "name": "pgste",
            "username": "pgste"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "b31510403b3e53f16abbb4f68e3e04d2032c5918",
          "message": "Merge pull request #52 from pgste/claude/reaper-security-perf-review-9pz54s",
          "timestamp": "2026-07-13T16:16:20+01:00",
          "tree_id": "b309c5a6777d66d3a23e2e72e71b6bb7b4015d28",
          "url": "https://github.com/pgste/reaper/commit/b31510403b3e53f16abbb4f68e3e04d2032c5918"
        },
        "date": 1783956086212,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_hot_swap/deploy_policy",
            "value": 147972,
            "range": "± 37140",
            "unit": "ns/iter"
          },
          {
            "name": "policy_hot_swap/concurrent_lookup",
            "value": 32,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "concurrent_access/concurrent_evaluations",
            "value": 410426,
            "range": "± 1306",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/100",
            "value": 172719,
            "range": "± 284",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/1000",
            "value": 3372625,
            "range": "± 124574",
            "unit": "ns/iter"
          },
          {
            "name": "realistic_workloads/microservice_auth_pattern",
            "value": 12038,
            "range": "± 30",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/has_relation_direct_hit",
            "value": 23,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/has_relation_direct_miss",
            "value": 23,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/reachable_1hop_hit",
            "value": 66,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/reachable_3hop_miss_bounded",
            "value": 65,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/inherited_1hop_hit",
            "value": 28,
            "range": "± 0",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}