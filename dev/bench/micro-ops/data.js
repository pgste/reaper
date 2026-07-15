window.BENCHMARK_DATA = {
  "lastUpdate": 1784145843602,
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
          "id": "dd90260d67a95dd0cc89a7d03d118e297d0509fd",
          "message": "Merge pull request #70 from pgste/claude/reaper-f1-capability-core",
          "timestamp": "2026-07-15T20:56:03+01:00",
          "tree_id": "3c304be15cf06c0b5d4ccce45de520943aa315ea",
          "url": "https://github.com/pgste/reaper/commit/dd90260d67a95dd0cc89a7d03d118e297d0509fd"
        },
        "date": 1784145843454,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_hot_swap/deploy_policy",
            "value": 182049,
            "range": "± 22362",
            "unit": "ns/iter"
          },
          {
            "name": "policy_hot_swap/concurrent_lookup",
            "value": 34,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "concurrent_access/concurrent_evaluations",
            "value": 332215,
            "range": "± 11414",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/100",
            "value": 182265,
            "range": "± 304",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/1000",
            "value": 3678555,
            "range": "± 3934",
            "unit": "ns/iter"
          },
          {
            "name": "realistic_workloads/microservice_auth_pattern",
            "value": 12688,
            "range": "± 178",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/has_relation_direct_hit",
            "value": 21,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/has_relation_direct_miss",
            "value": 21,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/reachable_1hop_hit",
            "value": 56,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/reachable_3hop_miss_bounded",
            "value": 56,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/inherited_1hop_hit",
            "value": 23,
            "range": "± 0",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}