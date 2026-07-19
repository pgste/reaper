window.BENCHMARK_DATA = {
  "lastUpdate": 1784489889760,
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
          "id": "abd233dcff60052c408f25dcb4b6ed75761ec567",
          "message": "Merge pull request #90 from pgste/claude/reaper-ci-bdd-gate",
          "timestamp": "2026-07-19T20:31:23+01:00",
          "tree_id": "9153198b9efc483ee871eea0a8c90b4b2d95af05",
          "url": "https://github.com/pgste/reaper/commit/abd233dcff60052c408f25dcb4b6ed75761ec567"
        },
        "date": 1784489888165,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 109,
            "range": "± 5",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 331,
            "range": "± 8",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 109,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 246,
            "range": "± 5",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 440,
            "range": "± 11",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 976,
            "range": "± 19",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}