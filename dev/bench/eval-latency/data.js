window.BENCHMARK_DATA = {
  "lastUpdate": 1783909985918,
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
          "id": "bfa920ae0ae8bf9c16734d00aa297ae042fc3f04",
          "message": "Merge pull request #48 from pgste/claude/reaper-security-perf-review-9pz54s",
          "timestamp": "2026-07-13T03:28:12+01:00",
          "tree_id": "6fd5601b289fc7068b09ea9090c195dde3817a3d",
          "url": "https://github.com/pgste/reaper/commit/bfa920ae0ae8bf9c16734d00aa297ae042fc3f04"
        },
        "date": 1783909984580,
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
            "value": 474,
            "range": "± 14",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 118,
            "range": "± 5",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 307,
            "range": "± 7",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 560,
            "range": "± 4",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1317,
            "range": "± 7",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}