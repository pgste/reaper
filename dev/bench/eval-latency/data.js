window.BENCHMARK_DATA = {
  "lastUpdate": 1784106631698,
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
          "id": "3177a96b4c94aa6ea818dc53bde0c318d81d8764",
          "message": "Merge pull request #63 from pgste/claude/reaper-e1-siem-connectors",
          "timestamp": "2026-07-15T10:03:05+01:00",
          "tree_id": "925b65b5de04afe428376cf8110f672c24d7c87e",
          "url": "https://github.com/pgste/reaper/commit/3177a96b4c94aa6ea818dc53bde0c318d81d8764"
        },
        "date": 1784106630447,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 119,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 456,
            "range": "± 2",
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
            "value": 302,
            "range": "± 12",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 556,
            "range": "± 9",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1308,
            "range": "± 24",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}