window.BENCHMARK_DATA = {
  "lastUpdate": 1783693628003,
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
          "id": "28bfa003ac71ec7a3bc76c9163586909cd00c0b8",
          "message": "Merge pull request #24 from pgste/claude/feat-availability-resilience",
          "timestamp": "2026-07-10T15:22:16+01:00",
          "tree_id": "1f7d424b62fb28bfcd0b49fcb4f18e677503ea80",
          "url": "https://github.com/pgste/reaper/commit/28bfa003ac71ec7a3bc76c9163586909cd00c0b8"
        },
        "date": 1783693627013,
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
            "value": 454,
            "range": "± 21",
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
            "value": 318,
            "range": "± 4",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 642,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1309,
            "range": "± 9",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}