window.BENCHMARK_DATA = {
  "lastUpdate": 1784158207466,
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
          "id": "65ba350aa19ce3bc8e8f3045eddaccbfe4037e4e",
          "message": "Merge pull request #72 from pgste/claude/reaper-f1-compiled-actor",
          "timestamp": "2026-07-16T00:25:17+01:00",
          "tree_id": "1285cae8b53318d6df0f21ea7eabc4838d848dad",
          "url": "https://github.com/pgste/reaper/commit/65ba350aa19ce3bc8e8f3045eddaccbfe4037e4e"
        },
        "date": 1784158206546,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 118,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 456,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 118,
            "range": "± 2",
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
            "value": 553,
            "range": "± 6",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1296,
            "range": "± 33",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}