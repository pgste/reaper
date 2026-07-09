window.BENCHMARK_DATA = {
  "lastUpdate": 1783640525231,
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
          "id": "2029149635b0fe11f5c538b6b752dea9051472fc",
          "message": "Merge pull request #20 from pgste/claude/feat-audit-mandatory-mode",
          "timestamp": "2026-07-10T00:37:16+01:00",
          "tree_id": "aad90a8842a4e6885864eb91383b27407eefc870",
          "url": "https://github.com/pgste/reaper/commit/2029149635b0fe11f5c538b6b752dea9051472fc"
        },
        "date": 1783640524352,
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
            "value": 472,
            "range": "± 3",
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
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 643,
            "range": "± 3",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1316,
            "range": "± 5",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}