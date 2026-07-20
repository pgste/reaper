window.BENCHMARK_DATA = {
  "lastUpdate": 1784541124137,
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
          "id": "4f8610d2a56d3f11372b329de33a1ecd96d77989",
          "message": "Merge pull request #97 from pgste/claude/reaper-slo-agentic",
          "timestamp": "2026-07-20T10:45:50+01:00",
          "tree_id": "8e9c1a06dd62c5c9838888168cd59c0c21f1aca0",
          "url": "https://github.com/pgste/reaper/commit/4f8610d2a56d3f11372b329de33a1ecd96d77989"
        },
        "date": 1784541124041,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_hot_swap/deploy_policy",
            "value": 138780,
            "range": "± 39119",
            "unit": "ns/iter"
          },
          {
            "name": "policy_hot_swap/concurrent_lookup",
            "value": 37,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "concurrent_access/concurrent_evaluations",
            "value": 182728,
            "range": "± 7136",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/100",
            "value": 131979,
            "range": "± 247",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/1000",
            "value": 2670763,
            "range": "± 9332",
            "unit": "ns/iter"
          },
          {
            "name": "realistic_workloads/microservice_auth_pattern",
            "value": 9511,
            "range": "± 461",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/has_relation_direct_hit",
            "value": 16,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/has_relation_direct_miss",
            "value": 16,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/reachable_1hop_hit",
            "value": 39,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/reachable_3hop_miss_bounded",
            "value": 39,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/inherited_1hop_hit",
            "value": 17,
            "range": "± 0",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}