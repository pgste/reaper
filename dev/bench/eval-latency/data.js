window.BENCHMARK_DATA = {
  "lastUpdate": 1783997749526,
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
          "id": "420171a6622c991350211b0759b368bd81739264",
          "message": "Merge pull request #59 from pgste/claude/reaper-security-perf-review-9pz54s",
          "timestamp": "2026-07-14T03:51:01+01:00",
          "tree_id": "a2c5e7be6bebdc735a77878d9cf3c3ed79b47970",
          "url": "https://github.com/pgste/reaper/commit/420171a6622c991350211b0759b368bd81739264"
        },
        "date": 1783997749066,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 131,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 451,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 131,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 311,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 594,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1294,
            "range": "± 21",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}