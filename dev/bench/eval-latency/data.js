window.BENCHMARK_DATA = {
  "lastUpdate": 1783972431526,
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
          "id": "a505c4bd1618ec9b60ff6c90b44f7c9a7e960096",
          "message": "Merge pull request #55 from pgste/claude/reaper-security-perf-review-9pz54s",
          "timestamp": "2026-07-13T20:49:09+01:00",
          "tree_id": "be4b4d429a27bc201941fd6281b5cc14fb70d109",
          "url": "https://github.com/pgste/reaper/commit/a505c4bd1618ec9b60ff6c90b44f7c9a7e960096"
        },
        "date": 1783972430980,
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
            "value": 473,
            "range": "± 4",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 119,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 305,
            "range": "± 3",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 555,
            "range": "± 8",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1314,
            "range": "± 8",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}