window.BENCHMARK_DATA = {
  "lastUpdate": 1783856681238,
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
          "id": "5247b9dbf8c7328d291454fa8473ba324cc310e3",
          "message": "Merge pull request #34 from pgste/claude/reaper-security-perf-review-9pz54s",
          "timestamp": "2026-07-12T12:37:26+01:00",
          "tree_id": "eefc378e9fd6b5357b8513bb5f6d5db35c1593db",
          "url": "https://github.com/pgste/reaper/commit/5247b9dbf8c7328d291454fa8473ba324cc310e3"
        },
        "date": 1783856680350,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 131,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 478,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 131,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 321,
            "range": "± 3",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 601,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1324,
            "range": "± 10",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}