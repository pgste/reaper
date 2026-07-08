window.BENCHMARK_DATA = {
  "lastUpdate": 1783539620944,
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
          "id": "e97cb95a045409b6ed34c433181ac2dda0af666a",
          "message": "Merge pull request #10 from pgste/claude/feat-authn-authz-foundation",
          "timestamp": "2026-07-08T20:35:21+01:00",
          "tree_id": "5350901f07bd5bb069d2d5a34af16c960b82102f",
          "url": "https://github.com/pgste/reaper/commit/e97cb95a045409b6ed34c433181ac2dda0af666a"
        },
        "date": 1783539619688,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 119,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 475,
            "range": "± 8",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 120,
            "range": "± 3",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 316,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 645,
            "range": "± 4",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1303,
            "range": "± 8",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}