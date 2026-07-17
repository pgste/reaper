window.BENCHMARK_DATA = {
  "lastUpdate": 1784279690390,
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
          "id": "bd477c00e3946d7427f3bc5c1e184b199c28f6ac",
          "message": "Merge pull request #81 from pgste/claude/reaper-plan04-dsl-contract",
          "timestamp": "2026-07-17T10:07:30+01:00",
          "tree_id": "d1e7f58023f3fae92b419b7df63a83fba86806d3",
          "url": "https://github.com/pgste/reaper/commit/bd477c00e3946d7427f3bc5c1e184b199c28f6ac"
        },
        "date": 1784279689828,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 133,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 470,
            "range": "± 9",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 134,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 355,
            "range": "± 5",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 622,
            "range": "± 5",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1354,
            "range": "± 36",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}