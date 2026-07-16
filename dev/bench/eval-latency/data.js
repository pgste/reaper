window.BENCHMARK_DATA = {
  "lastUpdate": 1784173053610,
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
          "id": "cc56cf254c8b8caac04574d7ab65d2f224aae957",
          "message": "Merge pull request #75 from pgste/claude/reaper-f1-mcp-adapter",
          "timestamp": "2026-07-16T04:32:42+01:00",
          "tree_id": "51f20ceab1d8fd5c8139f6d8f2261cad19e20dce",
          "url": "https://github.com/pgste/reaper/commit/cc56cf254c8b8caac04574d7ab65d2f224aae957"
        },
        "date": 1784173052083,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 120,
            "range": "± 3",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 459,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 121,
            "range": "± 3",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 304,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 564,
            "range": "± 4",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1307,
            "range": "± 11",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}