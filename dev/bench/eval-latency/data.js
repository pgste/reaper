window.BENCHMARK_DATA = {
  "lastUpdate": 1783900176951,
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
          "id": "78925f60b30444aff937a39d987d12a5ecf65b9f",
          "message": "Merge pull request #43 from pgste/claude/reaper-security-perf-review-9pz54s",
          "timestamp": "2026-07-13T00:42:15+01:00",
          "tree_id": "7b5821086d76a75c019138bbfc8fa4284a52c8cd",
          "url": "https://github.com/pgste/reaper/commit/78925f60b30444aff937a39d987d12a5ecf65b9f"
        },
        "date": 1783900175758,
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
            "value": 474,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 118,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 308,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 564,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1300,
            "range": "± 23",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}