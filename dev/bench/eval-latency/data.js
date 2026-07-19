window.BENCHMARK_DATA = {
  "lastUpdate": 1784492074046,
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
          "id": "8f297d90aab26fafd6a9c701654c1155f68a0ebc",
          "message": "Merge pull request #93 from pgste/claude/reaper-docker-pr-fastpath",
          "timestamp": "2026-07-19T21:09:49+01:00",
          "tree_id": "1dd2dd576670d057263bd8a941d5a2240cd9e1fa",
          "url": "https://github.com/pgste/reaper/commit/8f297d90aab26fafd6a9c701654c1155f68a0ebc"
        },
        "date": 1784492073551,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 122,
            "range": "± 3",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 448,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 122,
            "range": "± 5",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 304,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 561,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1291,
            "range": "± 7",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}