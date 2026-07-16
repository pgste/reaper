window.BENCHMARK_DATA = {
  "lastUpdate": 1784220592951,
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
          "id": "d574de06ad3c299b1acfb31840dcd88ea92143b1",
          "message": "Merge pull request #78 from pgste/claude/reaper-plan02-release-integrity",
          "timestamp": "2026-07-16T17:42:38+01:00",
          "tree_id": "260ef2c5a9c049518b797ddb987801e87aa821de",
          "url": "https://github.com/pgste/reaper/commit/d574de06ad3c299b1acfb31840dcd88ea92143b1"
        },
        "date": 1784220591755,
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
            "value": 445,
            "range": "± 21",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 123,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 304,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 559,
            "range": "± 7",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1314,
            "range": "± 36",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}