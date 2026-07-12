window.BENCHMARK_DATA = {
  "lastUpdate": 1783826444769,
  "repoUrl": "https://github.com/pgste/reaper",
  "entries": {
    "Eval latency (criterion)": [
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
          "id": "92f63c827845c4d49adf5acc9d45f531a0ef07c5",
          "message": "Merge pull request #33 from pgste/claude/reaper-security-perf-review-9pz54s",
          "timestamp": "2026-07-12T04:15:52+01:00",
          "tree_id": "137b8aceb34c059d4f819c37a351c32f92e8955d",
          "url": "https://github.com/pgste/reaper/commit/92f63c827845c4d49adf5acc9d45f531a0ef07c5"
        },
        "date": 1783826443237,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 111,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 354,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 112,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 318,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 648,
            "range": "± 5",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1113,
            "range": "± 4",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}