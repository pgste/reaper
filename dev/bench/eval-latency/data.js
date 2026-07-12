window.BENCHMARK_DATA = {
  "lastUpdate": 1783867146408,
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
          "id": "22624ed5cb3a0c012011d2e9cdc7e22f53f84e08",
          "message": "Merge pull request #36 from pgste/claude/reaper-security-perf-review-9pz54s",
          "timestamp": "2026-07-12T15:34:19+01:00",
          "tree_id": "77d33ce9872a988ba4d396770c5100a43c9ca605",
          "url": "https://github.com/pgste/reaper/commit/22624ed5cb3a0c012011d2e9cdc7e22f53f84e08"
        },
        "date": 1783867145188,
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
            "range": "± 5",
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
            "value": 299,
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
            "value": 1313,
            "range": "± 32",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}