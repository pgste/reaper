window.BENCHMARK_DATA = {
  "lastUpdate": 1783893038749,
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
          "id": "6ceac0cf3896034d9ecf46ac6fe683d7a7737555",
          "message": "Merge pull request #40 from pgste/claude/reaper-security-perf-review-9pz54s",
          "timestamp": "2026-07-12T22:46:01+01:00",
          "tree_id": "4c426b1a01be11e877f68737c9454ec23b1a67ab",
          "url": "https://github.com/pgste/reaper/commit/6ceac0cf3896034d9ecf46ac6fe683d7a7737555"
        },
        "date": 1783893038251,
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
            "value": 473,
            "range": "± 20",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 119,
            "range": "± 8",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 301,
            "range": "± 5",
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
            "value": 1277,
            "range": "± 20",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}