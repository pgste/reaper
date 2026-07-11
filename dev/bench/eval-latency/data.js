window.BENCHMARK_DATA = {
  "lastUpdate": 1783792244910,
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
          "id": "0a846136f75cb7a397ce8ce1d5360e4c4f74c49b",
          "message": "Merge pull request #32 from pgste/claude/docs-plan07-shipped",
          "timestamp": "2026-07-11T18:43:24+01:00",
          "tree_id": "16469ab78ca496459c0fc4913ba45062b5e3b91c",
          "url": "https://github.com/pgste/reaper/commit/0a846136f75cb7a397ce8ce1d5360e4c4f74c49b"
        },
        "date": 1783792243686,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 118,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 471,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 118,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 317,
            "range": "± 3",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 649,
            "range": "± 6",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1289,
            "range": "± 4",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}