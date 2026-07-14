window.BENCHMARK_DATA = {
  "lastUpdate": 1784049420349,
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
          "id": "70f973c13068340fd1cb7e7a78525fd339e03c6e",
          "message": "Merge pull request #60 from pgste/claude/reaper-security-perf-review-9pz54s",
          "timestamp": "2026-07-14T18:09:46+01:00",
          "tree_id": "8c8b0a90ee1384753799fa37fb9b407039dfdcaa",
          "url": "https://github.com/pgste/reaper/commit/70f973c13068340fd1cb7e7a78525fd339e03c6e"
        },
        "date": 1784049419460,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 131,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 438,
            "range": "± 3",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 130,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 314,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 595,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1287,
            "range": "± 13",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}