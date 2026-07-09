window.BENCHMARK_DATA = {
  "lastUpdate": 1783564406146,
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
          "id": "ebeae26b9ecc9ccfc3ec0e94df1dbb372bf7a4de",
          "message": "Merge pull request #14 from pgste/claude/feat-policy-integrity",
          "timestamp": "2026-07-09T03:28:25+01:00",
          "tree_id": "7862560ff7bdb09ab13bb8471b8b711e152ec411",
          "url": "https://github.com/pgste/reaper/commit/ebeae26b9ecc9ccfc3ec0e94df1dbb372bf7a4de"
        },
        "date": 1783564405606,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 131,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 467,
            "range": "± 7",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 132,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 357,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 688,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1414,
            "range": "± 8",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}