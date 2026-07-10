window.BENCHMARK_DATA = {
  "lastUpdate": 1783693638900,
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
          "id": "28bfa003ac71ec7a3bc76c9163586909cd00c0b8",
          "message": "Merge pull request #24 from pgste/claude/feat-availability-resilience",
          "timestamp": "2026-07-10T15:22:16+01:00",
          "tree_id": "1f7d424b62fb28bfcd0b49fcb4f18e677503ea80",
          "url": "https://github.com/pgste/reaper/commit/28bfa003ac71ec7a3bc76c9163586909cd00c0b8"
        },
        "date": 1783693638417,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_hot_swap/deploy_policy",
            "value": 152417,
            "range": "± 37318",
            "unit": "ns/iter"
          },
          {
            "name": "policy_hot_swap/concurrent_lookup",
            "value": 32,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "concurrent_access/concurrent_evaluations",
            "value": 407315,
            "range": "± 2086",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/100",
            "value": 144649,
            "range": "± 216",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/1000",
            "value": 3095739,
            "range": "± 3425",
            "unit": "ns/iter"
          },
          {
            "name": "realistic_workloads/microservice_auth_pattern",
            "value": 11846,
            "range": "± 35",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/has_relation_direct_hit",
            "value": 23,
            "range": "± 0",
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
            "value": 156,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/reachable_3hop_miss_bounded",
            "value": 315,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/inherited_1hop_hit",
            "value": 141,
            "range": "± 1",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}