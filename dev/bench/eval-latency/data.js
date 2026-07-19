window.BENCHMARK_DATA = {
  "lastUpdate": 1784458488922,
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
          "id": "ee841db3c3adf9c6e0bf7fcc38a7372a16244631",
          "message": "Merge pull request #87 from pgste/claude/reaper-plan06-pagination-b3",
          "timestamp": "2026-07-19T11:47:40+01:00",
          "tree_id": "3ca81e93eac84c8234348fa2c6f593e8a2cd22e7",
          "url": "https://github.com/pgste/reaper/commit/ee841db3c3adf9c6e0bf7fcc38a7372a16244631"
        },
        "date": 1784458487986,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 121,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 447,
            "range": "± 10",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 121,
            "range": "± 1",
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
            "value": 567,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1309,
            "range": "± 8",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}