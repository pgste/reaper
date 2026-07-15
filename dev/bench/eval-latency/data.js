window.BENCHMARK_DATA = {
  "lastUpdate": 1784131044691,
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
          "id": "d896dc3650cf7f3d9b102a04fdae4e73eceee021",
          "message": "Merge pull request #67 from pgste/claude/reaper-f2-wasm-target",
          "timestamp": "2026-07-15T16:52:51+01:00",
          "tree_id": "f1fc57a7a812e4e23c79dd74a83fbce8e7e3a649",
          "url": "https://github.com/pgste/reaper/commit/d896dc3650cf7f3d9b102a04fdae4e73eceee021"
        },
        "date": 1784131043320,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 111,
            "range": "± 4",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 362,
            "range": "± 18",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 113,
            "range": "± 7",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 263,
            "range": "± 14",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 473,
            "range": "± 22",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1012,
            "range": "± 50",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}