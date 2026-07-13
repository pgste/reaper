window.BENCHMARK_DATA = {
  "lastUpdate": 1783940436718,
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
          "id": "c801f037bd48661f0abe2767320b6f47b5993caf",
          "message": "Merge pull request #49 from pgste/claude/reaper-security-perf-review-9pz54s",
          "timestamp": "2026-07-13T11:53:25+01:00",
          "tree_id": "9e3380435ead58b10d2649647c5699ac7ab7df5b",
          "url": "https://github.com/pgste/reaper/commit/c801f037bd48661f0abe2767320b6f47b5993caf"
        },
        "date": 1783940436113,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 119,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 471,
            "range": "± 13",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 118,
            "range": "± 6",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 300,
            "range": "± 0",
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
            "value": 1303,
            "range": "± 8",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}