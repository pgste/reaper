window.BENCHMARK_DATA = {
  "lastUpdate": 1784224134972,
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
          "id": "7f6988ae8ffa4c0247f45903c536789fe75f8e4e",
          "message": "Merge pull request #79 from pgste/claude/reaper-enterprise-review-mlwzsk",
          "timestamp": "2026-07-16T18:44:16+01:00",
          "tree_id": "e340cff98be65b3f48963ee7df8ac103a8618a1a",
          "url": "https://github.com/pgste/reaper/commit/7f6988ae8ffa4c0247f45903c536789fe75f8e4e"
        },
        "date": 1784224134490,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 121,
            "range": "± 5",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 447,
            "range": "± 4",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 122,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 302,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 556,
            "range": "± 3",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1286,
            "range": "± 10",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}