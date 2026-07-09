window.BENCHMARK_DATA = {
  "lastUpdate": 1783634124111,
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
          "id": "ce1da2ebcf9bab8a56056f7dca91477d4a52dc14",
          "message": "Merge pull request #19 from pgste/claude/feat-audit-checkpoints",
          "timestamp": "2026-07-09T22:50:37+01:00",
          "tree_id": "30bada562370d2d17c10c25c3c366d359a8f7a89",
          "url": "https://github.com/pgste/reaper/commit/ce1da2ebcf9bab8a56056f7dca91477d4a52dc14"
        },
        "date": 1783634123345,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 111,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 352,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 111,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 307,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 645,
            "range": "± 3",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1164,
            "range": "± 21",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}