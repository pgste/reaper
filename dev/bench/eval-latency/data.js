window.BENCHMARK_DATA = {
  "lastUpdate": 1784137514294,
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
          "id": "7c0f6dc190220b15f8f1124948d58b299f62f564",
          "message": "Merge pull request #68 from pgste/claude/reaper-f2-wasm-cdylib",
          "timestamp": "2026-07-15T18:37:56+01:00",
          "tree_id": "ea467a7be1652aa09f342c48129537e86d81213e",
          "url": "https://github.com/pgste/reaper/commit/7c0f6dc190220b15f8f1124948d58b299f62f564"
        },
        "date": 1784137513072,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 119,
            "range": "± 4",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 479,
            "range": "± 11",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 120,
            "range": "± 9",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 308,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 556,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1301,
            "range": "± 9",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}