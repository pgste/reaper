window.BENCHMARK_DATA = {
  "lastUpdate": 1783544210019,
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
          "id": "18d205f9d45608a789af5806542451f54d261080",
          "message": "Merge pull request #11 from pgste/claude/feat-authn-authz-foundation",
          "timestamp": "2026-07-08T21:52:25+01:00",
          "tree_id": "2f3d0b83b3a59e8a9ed5a52d2f9f57e20b3e41a0",
          "url": "https://github.com/pgste/reaper/commit/18d205f9d45608a789af5806542451f54d261080"
        },
        "date": 1783544208703,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 102,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 324,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 101,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 280,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 534,
            "range": "± 6",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1032,
            "range": "± 4",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}