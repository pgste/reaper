window.BENCHMARK_DATA = {
  "lastUpdate": 1784121017379,
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
          "id": "256e3444aad6ba160bfba7fc814b029acc2d3b01",
          "message": "Merge pull request #65 from pgste/claude/reaper-e3-airgap-signing",
          "timestamp": "2026-07-15T14:05:24+01:00",
          "tree_id": "4bc75b78916779bb7342f7e67b1730f6d1cff05f",
          "url": "https://github.com/pgste/reaper/commit/256e3444aad6ba160bfba7fc814b029acc2d3b01"
        },
        "date": 1784121016534,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 130,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 469,
            "range": "± 3",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 131,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 313,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 604,
            "range": "± 13",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1288,
            "range": "± 11",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}