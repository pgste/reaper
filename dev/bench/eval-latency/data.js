window.BENCHMARK_DATA = {
  "lastUpdate": 1783961460497,
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
          "id": "9056295246e11276d5bb18b0eba3fc16fc8ad855",
          "message": "Merge pull request #53 from pgste/claude/reaper-security-perf-review-9pz54s",
          "timestamp": "2026-07-13T17:46:17+01:00",
          "tree_id": "7bd408fdf17d891a032272d991cdc5208f28f7e5",
          "url": "https://github.com/pgste/reaper/commit/9056295246e11276d5bb18b0eba3fc16fc8ad855"
        },
        "date": 1783961459931,
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
            "value": 480,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 118,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 300,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 557,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1307,
            "range": "± 14",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}