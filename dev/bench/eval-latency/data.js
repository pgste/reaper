window.BENCHMARK_DATA = {
  "lastUpdate": 1783904203701,
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
          "id": "19fe843332de2d7d2a0c178916366403e558b28a",
          "message": "Merge pull request #45 from pgste/claude/reaper-security-perf-review-9pz54s",
          "timestamp": "2026-07-13T01:52:00+01:00",
          "tree_id": "9e9b67bba70daa42c51f7f7ba4cfc4d58398644e",
          "url": "https://github.com/pgste/reaper/commit/19fe843332de2d7d2a0c178916366403e558b28a"
        },
        "date": 1783904203189,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 120,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 473,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 118,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 301,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 556,
            "range": "± 4",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1321,
            "range": "± 15",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}