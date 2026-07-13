window.BENCHMARK_DATA = {
  "lastUpdate": 1783965949892,
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
          "id": "763ee13d8c0b6e0b4ac44a27c57c96802ebe30a8",
          "message": "Merge pull request #54 from pgste/claude/reaper-security-perf-review-9pz54s",
          "timestamp": "2026-07-13T19:01:00+01:00",
          "tree_id": "01edeade8702dd4910a28553b8743b642d44e0e2",
          "url": "https://github.com/pgste/reaper/commit/763ee13d8c0b6e0b4ac44a27c57c96802ebe30a8"
        },
        "date": 1783965949336,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 119,
            "range": "± 3",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 508,
            "range": "± 3",
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
            "value": 304,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 557,
            "range": "± 20",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1318,
            "range": "± 7",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}