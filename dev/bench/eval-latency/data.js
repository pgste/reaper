window.BENCHMARK_DATA = {
  "lastUpdate": 1783779744926,
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
          "id": "1f756457387b98eb6287803f349386f1dfb5c2bc",
          "message": "Merge pull request #30 from pgste/claude/feat-api-governance-idempotency",
          "timestamp": "2026-07-11T15:17:39+01:00",
          "tree_id": "cb13bde3b90b160e3dcdf4205d84ea753bf58e80",
          "url": "https://github.com/pgste/reaper/commit/1f756457387b98eb6287803f349386f1dfb5c2bc"
        },
        "date": 1783779744135,
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
            "range": "± 4",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 326,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 648,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1317,
            "range": "± 32",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}