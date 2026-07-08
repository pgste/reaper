window.BENCHMARK_DATA = {
  "lastUpdate": 1783539466841,
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
          "id": "2e5c28c25d59e9aa3adebddbe0800febad59a5a6",
          "message": "Merge pull request #9 from pgste/claude/reaper-security-perf-review-9pz54s",
          "timestamp": "2026-07-08T20:32:56+01:00",
          "tree_id": "c726a7f30abfeeba39a2c26ccef8a530ca28f619",
          "url": "https://github.com/pgste/reaper/commit/2e5c28c25d59e9aa3adebddbe0800febad59a5a6"
        },
        "date": 1783539465661,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 119,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 474,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 119,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 317,
            "range": "± 4",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 645,
            "range": "± 6",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1280,
            "range": "± 8",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}