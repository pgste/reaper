window.BENCHMARK_DATA = {
  "lastUpdate": 1784479355617,
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
          "id": "7a6255645b3d628b3684f884460986c63cd79651",
          "message": "Merge pull request #89 from pgste/claude/reaper-plan06-prunability",
          "timestamp": "2026-07-19T17:37:38+01:00",
          "tree_id": "30c383bf3203fe4df1816bbda831cc17c340cd88",
          "url": "https://github.com/pgste/reaper/commit/7a6255645b3d628b3684f884460986c63cd79651"
        },
        "date": 1784479354272,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 122,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 463,
            "range": "± 9",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 122,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 310,
            "range": "± 3",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 569,
            "range": "± 6",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1318,
            "range": "± 35",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}