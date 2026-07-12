window.BENCHMARK_DATA = {
  "lastUpdate": 1783895446699,
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
          "id": "f300b6b04ed8fc418d2638c2659512f330e60e18",
          "message": "Merge pull request #42 from pgste/claude/reaper-security-perf-review-9pz54s",
          "timestamp": "2026-07-12T23:26:06+01:00",
          "tree_id": "be9b3d70ebeee3ea7198176e8a8bb28071480480",
          "url": "https://github.com/pgste/reaper/commit/f300b6b04ed8fc418d2638c2659512f330e60e18"
        },
        "date": 1783895445918,
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
            "value": 470,
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
            "value": 306,
            "range": "± 7",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 553,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1322,
            "range": "± 9",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}