window.BENCHMARK_DATA = {
  "lastUpdate": 1783566787853,
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
          "id": "acef8df3b7b4d04b32582c09cf6ffe82fcc0a94d",
          "message": "Merge pull request #15 from pgste/claude/ci-sccache-optimization",
          "timestamp": "2026-07-09T04:07:51+01:00",
          "tree_id": "e130c888e220d354b27cc23fe93440f2aee7ba30",
          "url": "https://github.com/pgste/reaper/commit/acef8df3b7b4d04b32582c09cf6ffe82fcc0a94d"
        },
        "date": 1783566787323,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_hot_swap/deploy_policy",
            "value": 154319,
            "range": "± 37528",
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
            "value": 374574,
            "range": "± 2175",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/100",
            "value": 147222,
            "range": "± 3404",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/1000",
            "value": 3167010,
            "range": "± 7220",
            "unit": "ns/iter"
          },
          {
            "name": "realistic_workloads/microservice_auth_pattern",
            "value": 12023,
            "range": "± 52",
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
            "value": 313,
            "range": "± 12",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/inherited_1hop_hit",
            "value": 141,
            "range": "± 0",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}