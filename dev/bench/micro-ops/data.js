window.BENCHMARK_DATA = {
  "lastUpdate": 1783644106193,
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
          "id": "9e3cda4db9b9cb34344a05c3197fa3c18b7ad02d",
          "message": "Merge pull request #21 from pgste/claude/feat-audit-retention",
          "timestamp": "2026-07-10T01:36:36+01:00",
          "tree_id": "14a1233815c3c09d86e1a55ac61e7759326d20d7",
          "url": "https://github.com/pgste/reaper/commit/9e3cda4db9b9cb34344a05c3197fa3c18b7ad02d"
        },
        "date": 1783644105685,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_hot_swap/deploy_policy",
            "value": 152265,
            "range": "± 37087",
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
            "value": 378543,
            "range": "± 8675",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/100",
            "value": 144481,
            "range": "± 389",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/1000",
            "value": 3100894,
            "range": "± 6936",
            "unit": "ns/iter"
          },
          {
            "name": "realistic_workloads/microservice_auth_pattern",
            "value": 12020,
            "range": "± 33",
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
            "value": 157,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/reachable_3hop_miss_bounded",
            "value": 309,
            "range": "± 8",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/inherited_1hop_hit",
            "value": 140,
            "range": "± 6",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}