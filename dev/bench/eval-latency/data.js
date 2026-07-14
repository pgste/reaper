window.BENCHMARK_DATA = {
  "lastUpdate": 1783989007055,
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
          "id": "e1f4b8979b2d479235fead52fa20f3027c0ee2bc",
          "message": "Merge pull request #58 from pgste/claude/reaper-security-perf-review-9pz54s",
          "timestamp": "2026-07-14T01:25:24+01:00",
          "tree_id": "91b52e9e3540e6991f65b72b1e4014ab058c0558",
          "url": "https://github.com/pgste/reaper/commit/e1f4b8979b2d479235fead52fa20f3027c0ee2bc"
        },
        "date": 1783989005822,
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
            "value": 476,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 119,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 304,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 553,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1307,
            "range": "± 10",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}