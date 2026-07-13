window.BENCHMARK_DATA = {
  "lastUpdate": 1783909749151,
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
          "id": "d8719da7c1196f243de8fabe34ebc4f0c85735da",
          "message": "Merge pull request #47 from pgste/claude/reaper-security-perf-review-9pz54s",
          "timestamp": "2026-07-13T03:24:21+01:00",
          "tree_id": "b4efe0de9446093c92ce2e3a9cfb74b750203eef",
          "url": "https://github.com/pgste/reaper/commit/d8719da7c1196f243de8fabe34ebc4f0c85735da"
        },
        "date": 1783909748676,
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
            "value": 472,
            "range": "± 1",
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
            "value": 302,
            "range": "± 5",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 562,
            "range": "± 4",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1308,
            "range": "± 9",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}