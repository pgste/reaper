window.BENCHMARK_DATA = {
  "lastUpdate": 1783792293700,
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
          "id": "0a846136f75cb7a397ce8ce1d5360e4c4f74c49b",
          "message": "Merge pull request #32 from pgste/claude/docs-plan07-shipped",
          "timestamp": "2026-07-11T18:43:24+01:00",
          "tree_id": "16469ab78ca496459c0fc4913ba45062b5e3b91c",
          "url": "https://github.com/pgste/reaper/commit/0a846136f75cb7a397ce8ce1d5360e4c4f74c49b"
        },
        "date": 1783792293212,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_hot_swap/deploy_policy",
            "value": 150119,
            "range": "± 37595",
            "unit": "ns/iter"
          },
          {
            "name": "policy_hot_swap/concurrent_lookup",
            "value": 33,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "concurrent_access/concurrent_evaluations",
            "value": 420615,
            "range": "± 3390",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/100",
            "value": 147601,
            "range": "± 577",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/1000",
            "value": 3090400,
            "range": "± 8120",
            "unit": "ns/iter"
          },
          {
            "name": "realistic_workloads/microservice_auth_pattern",
            "value": 11910,
            "range": "± 51",
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
            "value": 311,
            "range": "± 4",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/inherited_1hop_hit",
            "value": 142,
            "range": "± 0",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}