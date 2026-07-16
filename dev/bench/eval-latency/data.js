window.BENCHMARK_DATA = {
  "lastUpdate": 1784204862915,
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
          "id": "9ff07b6dcb087e690eec5d1e099a527608b110fa",
          "message": "Merge pull request #76 from pgste/claude/reaper-enterprise-review-mlwzsk",
          "timestamp": "2026-07-16T13:20:34+01:00",
          "tree_id": "a61d50ca1f9ac8b06bd8f9b396e073f31e2465fa",
          "url": "https://github.com/pgste/reaper/commit/9ff07b6dcb087e690eec5d1e099a527608b110fa"
        },
        "date": 1784204861962,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 120,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 462,
            "range": "± 2",
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
            "value": 303,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 557,
            "range": "± 5",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1285,
            "range": "± 29",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}