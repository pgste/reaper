window.BENCHMARK_DATA = {
  "lastUpdate": 1783560786502,
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
          "id": "47c96cae66298f9c4ad9a894765ed09017d7247d",
          "message": "Merge pull request #13 from pgste/claude/feat-policy-integrity",
          "timestamp": "2026-07-09T02:26:00+01:00",
          "tree_id": "cfa8f77b19e7bfe62f4d266dd1758b0d39ff49b9",
          "url": "https://github.com/pgste/reaper/commit/47c96cae66298f9c4ad9a894765ed09017d7247d"
        },
        "date": 1783560786011,
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
            "value": 480,
            "range": "± 5",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 120,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 315,
            "range": "± 15",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 646,
            "range": "± 9",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1303,
            "range": "± 11",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}