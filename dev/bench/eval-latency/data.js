window.BENCHMARK_DATA = {
  "lastUpdate": 1783862917638,
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
          "id": "f2855e4fe4bd7511e902b7b182cd8280d1797111",
          "message": "Merge pull request #35 from pgste/claude/reaper-security-perf-review-9pz54s",
          "timestamp": "2026-07-12T14:23:57+01:00",
          "tree_id": "b5016aac63f7d0d976bc647a29394f4cd84cb0db",
          "url": "https://github.com/pgste/reaper/commit/f2855e4fe4bd7511e902b7b182cd8280d1797111"
        },
        "date": 1783862917139,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 119,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 475,
            "range": "± 4",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 119,
            "range": "± 3",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 302,
            "range": "± 5",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 560,
            "range": "± 5",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1321,
            "range": "± 11",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}