window.BENCHMARK_DATA = {
  "lastUpdate": 1783566769750,
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
          "id": "acef8df3b7b4d04b32582c09cf6ffe82fcc0a94d",
          "message": "Merge pull request #15 from pgste/claude/ci-sccache-optimization",
          "timestamp": "2026-07-09T04:07:51+01:00",
          "tree_id": "e130c888e220d354b27cc23fe93440f2aee7ba30",
          "url": "https://github.com/pgste/reaper/commit/acef8df3b7b4d04b32582c09cf6ffe82fcc0a94d"
        },
        "date": 1783566768550,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 119,
            "range": "± 3",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 476,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 120,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 317,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 646,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1318,
            "range": "± 8",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}