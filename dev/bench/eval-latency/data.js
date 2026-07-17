window.BENCHMARK_DATA = {
  "lastUpdate": 1784284543002,
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
          "id": "60a30effee0dd0e59f43d082f183aef458b3738b",
          "message": "Merge pull request #82 from pgste/claude/reaper-plan05-verification-gates",
          "timestamp": "2026-07-17T11:30:42+01:00",
          "tree_id": "5abb824bb33c55f72c4e2d44caec53335c36d565",
          "url": "https://github.com/pgste/reaper/commit/60a30effee0dd0e59f43d082f183aef458b3738b"
        },
        "date": 1784284541594,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 133,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 477,
            "range": "± 3",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 134,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 333,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 595,
            "range": "± 3",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1383,
            "range": "± 43",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}