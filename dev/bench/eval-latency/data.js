window.BENCHMARK_DATA = {
  "lastUpdate": 1783785122454,
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
          "id": "0909df06b3acea7b83a44ad37b010195e4abd480",
          "message": "Merge pull request #31 from pgste/claude/feat-api-governance-pagination-errors",
          "timestamp": "2026-07-11T16:47:25+01:00",
          "tree_id": "36477374906007c813de5f3b19c6d92a96481137",
          "url": "https://github.com/pgste/reaper/commit/0909df06b3acea7b83a44ad37b010195e4abd480"
        },
        "date": 1783785121583,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 108,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 345,
            "range": "± 9",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 109,
            "range": "± 4",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 265,
            "range": "± 19",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 536,
            "range": "± 11",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1015,
            "range": "± 52",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}