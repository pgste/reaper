window.BENCHMARK_DATA = {
  "lastUpdate": 1783627749137,
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
          "id": "58e4e2bfabf9bb4bb07be829046a564d3704c49c",
          "message": "Merge pull request #18 from pgste/claude/feat-audit-integrity",
          "timestamp": "2026-07-09T21:04:16+01:00",
          "tree_id": "b0258e916a6bb3a5b831beaad6de4a19c06af526",
          "url": "https://github.com/pgste/reaper/commit/58e4e2bfabf9bb4bb07be829046a564d3704c49c"
        },
        "date": 1783627747707,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 111,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 353,
            "range": "± 7",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 111,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 307,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 644,
            "range": "± 4",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1171,
            "range": "± 17",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}