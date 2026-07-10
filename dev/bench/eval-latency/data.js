window.BENCHMARK_DATA = {
  "lastUpdate": 1783644086709,
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
          "id": "9e3cda4db9b9cb34344a05c3197fa3c18b7ad02d",
          "message": "Merge pull request #21 from pgste/claude/feat-audit-retention",
          "timestamp": "2026-07-10T01:36:36+01:00",
          "tree_id": "14a1233815c3c09d86e1a55ac61e7759326d20d7",
          "url": "https://github.com/pgste/reaper/commit/9e3cda4db9b9cb34344a05c3197fa3c18b7ad02d"
        },
        "date": 1783644085524,
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
            "value": 473,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 118,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 321,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 645,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1346,
            "range": "± 7",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}