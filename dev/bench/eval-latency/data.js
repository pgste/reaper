window.BENCHMARK_DATA = {
  "lastUpdate": 1784541122663,
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
          "id": "4f8610d2a56d3f11372b329de33a1ecd96d77989",
          "message": "Merge pull request #97 from pgste/claude/reaper-slo-agentic",
          "timestamp": "2026-07-20T10:45:50+01:00",
          "tree_id": "8e9c1a06dd62c5c9838888168cd59c0c21f1aca0",
          "url": "https://github.com/pgste/reaper/commit/4f8610d2a56d3f11372b329de33a1ecd96d77989"
        },
        "date": 1784541121848,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 93,
            "range": "± 7",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 227,
            "range": "± 10",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 93,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 210,
            "range": "± 11",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 358,
            "range": "± 5",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 813,
            "range": "± 42",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}