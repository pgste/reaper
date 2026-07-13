window.BENCHMARK_DATA = {
  "lastUpdate": 1783956073786,
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
          "id": "b31510403b3e53f16abbb4f68e3e04d2032c5918",
          "message": "Merge pull request #52 from pgste/claude/reaper-security-perf-review-9pz54s",
          "timestamp": "2026-07-13T16:16:20+01:00",
          "tree_id": "b309c5a6777d66d3a23e2e72e71b6bb7b4015d28",
          "url": "https://github.com/pgste/reaper/commit/b31510403b3e53f16abbb4f68e3e04d2032c5918"
        },
        "date": 1783956072362,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 119,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 485,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 118,
            "range": "± 4",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 305,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 556,
            "range": "± 7",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1297,
            "range": "± 25",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}