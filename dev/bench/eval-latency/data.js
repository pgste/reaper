window.BENCHMARK_DATA = {
  "lastUpdate": 1784415567002,
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
          "id": "7f70a584da7d31baf06fe051f742b6c3af76d551",
          "message": "Merge pull request #86 from pgste/claude/reaper-plan06-pagination",
          "timestamp": "2026-07-18T23:54:43+01:00",
          "tree_id": "35969068148f95b3d0bf3ff4c99defdcee896875",
          "url": "https://github.com/pgste/reaper/commit/7f70a584da7d31baf06fe051f742b6c3af76d551"
        },
        "date": 1784415566505,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 121,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 449,
            "range": "± 5",
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
            "value": 308,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 570,
            "range": "± 5",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1352,
            "range": "± 8",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}