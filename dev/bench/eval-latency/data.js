window.BENCHMARK_DATA = {
  "lastUpdate": 1783901051279,
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
          "id": "f99a58bdfe1b2568f7746fbc79f269cc4a460ddd",
          "message": "Merge pull request #44 from pgste/claude/reaper-security-perf-review-9pz54s",
          "timestamp": "2026-07-13T00:59:29+01:00",
          "tree_id": "29475fb1df33a91b464ad305ed294d3b590b7aa4",
          "url": "https://github.com/pgste/reaper/commit/f99a58bdfe1b2568f7746fbc79f269cc4a460ddd"
        },
        "date": 1783901050767,
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
            "value": 476,
            "range": "± 3",
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
            "value": 300,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 558,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1304,
            "range": "± 11",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}