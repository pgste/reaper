window.BENCHMARK_DATA = {
  "lastUpdate": 1783976473786,
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
          "id": "dd631427362ccf6dea38883ff810d716a61dfecd",
          "message": "Merge pull request #56 from pgste/claude/reaper-security-perf-review-9pz54s",
          "timestamp": "2026-07-13T21:56:31+01:00",
          "tree_id": "324e548a0859f3fd7232d398f70e042a41c79a58",
          "url": "https://github.com/pgste/reaper/commit/dd631427362ccf6dea38883ff810d716a61dfecd"
        },
        "date": 1783976473278,
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
            "value": 471,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 119,
            "range": "± 5",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 303,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 554,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1326,
            "range": "± 8",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}