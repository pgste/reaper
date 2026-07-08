window.BENCHMARK_DATA = {
  "lastUpdate": 1783547573515,
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
          "id": "77a284475451ac39a803a2638cb24c22437545c2",
          "message": "Merge pull request #12 from pgste/claude/feat-policy-integrity",
          "timestamp": "2026-07-08T22:48:10+01:00",
          "tree_id": "08c244c4100bac6bac6d5487819b7b374d6791fe",
          "url": "https://github.com/pgste/reaper/commit/77a284475451ac39a803a2638cb24c22437545c2"
        },
        "date": 1783547573041,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 120,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 475,
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
            "value": 327,
            "range": "± 6",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 637,
            "range": "± 3",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1284,
            "range": "± 15",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}