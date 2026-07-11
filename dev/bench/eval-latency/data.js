window.BENCHMARK_DATA = {
  "lastUpdate": 1783732424011,
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
          "id": "342d0d022c2fd83750182bf0c6c258eb567d9c6f",
          "message": "Merge pull request #26 from pgste/claude/deps-latest-upgrades",
          "timestamp": "2026-07-11T02:06:20+01:00",
          "tree_id": "3d9b18563acdf866cb4e3390353997e9cef3bf69",
          "url": "https://github.com/pgste/reaper/commit/342d0d022c2fd83750182bf0c6c258eb567d9c6f"
        },
        "date": 1783732422739,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 118,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 471,
            "range": "± 6",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 119,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 321,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 634,
            "range": "± 21",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1289,
            "range": "± 7",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}