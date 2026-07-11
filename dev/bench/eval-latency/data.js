window.BENCHMARK_DATA = {
  "lastUpdate": 1783736472843,
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
          "id": "e28735aaac8d0b19353472b7600d75b1c064d8ea",
          "message": "Merge pull request #27 from pgste/claude/feat-api-governance-openapi",
          "timestamp": "2026-07-11T03:16:18+01:00",
          "tree_id": "e08014d70fbece045c2cdccb463aa4b29eda19b9",
          "url": "https://github.com/pgste/reaper/commit/e28735aaac8d0b19353472b7600d75b1c064d8ea"
        },
        "date": 1783736471588,
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
            "value": 318,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 636,
            "range": "± 3",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1372,
            "range": "± 67",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}