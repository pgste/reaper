window.BENCHMARK_DATA = {
  "lastUpdate": 1783770993101,
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
          "id": "20baeb4cbc586ed264baabecd6a73f02385f3526",
          "message": "Merge pull request #28 from pgste/claude/feat-api-governance-v1-surface",
          "timestamp": "2026-07-11T12:49:29+01:00",
          "tree_id": "798970d997caf03bc7b1065a928b37ea7b554b8a",
          "url": "https://github.com/pgste/reaper/commit/20baeb4cbc586ed264baabecd6a73f02385f3526"
        },
        "date": 1783770992271,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 118,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 471,
            "range": "± 14",
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
            "value": 318,
            "range": "± 3",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 642,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1357,
            "range": "± 7",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}