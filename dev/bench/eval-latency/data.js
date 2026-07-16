window.BENCHMARK_DATA = {
  "lastUpdate": 1784169722254,
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
          "id": "87ce5350d17a873c6d86aae8e688a80239af8716",
          "message": "Merge pull request #74 from pgste/claude/reaper-f1-allow-explain",
          "timestamp": "2026-07-16T03:37:16+01:00",
          "tree_id": "955b686014f7214a11042114b9f62b3882c14e47",
          "url": "https://github.com/pgste/reaper/commit/87ce5350d17a873c6d86aae8e688a80239af8716"
        },
        "date": 1784169721144,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 120,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 461,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 121,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 305,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 563,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1307,
            "range": "± 27",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}