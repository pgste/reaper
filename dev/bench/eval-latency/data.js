window.BENCHMARK_DATA = {
  "lastUpdate": 1784148392338,
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
          "id": "b8a7d4fd3eaa8889260d27ff3ce9e945811ec7ac",
          "message": "Merge pull request #71 from pgste/claude/reaper-f1-actor-dsl",
          "timestamp": "2026-07-15T21:41:51+01:00",
          "tree_id": "533521abcfb7a66b1492e4a548700e8662f46304",
          "url": "https://github.com/pgste/reaper/commit/b8a7d4fd3eaa8889260d27ff3ce9e945811ec7ac"
        },
        "date": 1784148391845,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 118,
            "range": "± 3",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 459,
            "range": "± 3",
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
            "value": 301,
            "range": "± 7",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 564,
            "range": "± 7",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1324,
            "range": "± 15",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}