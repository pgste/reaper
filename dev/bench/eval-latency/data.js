window.BENCHMARK_DATA = {
  "lastUpdate": 1784078984643,
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
          "id": "13623b4644645146e209be4d85469f558b3d43d3",
          "message": "Merge pull request #62 from pgste/claude/reaper-e2-erasure-followups-cq3c2o",
          "timestamp": "2026-07-15T02:23:41+01:00",
          "tree_id": "736eac55da8a684835a2703c29b9e902f37083b4",
          "url": "https://github.com/pgste/reaper/commit/13623b4644645146e209be4d85469f558b3d43d3"
        },
        "date": 1784078983889,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 90,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 236,
            "range": "± 14",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 90,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 204,
            "range": "± 9",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 370,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 764,
            "range": "± 4",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}