window.BENCHMARK_DATA = {
  "lastUpdate": 1783598719805,
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
          "id": "cf6ed1e55fc260260991bdfd571b113cde201abe",
          "message": "Merge pull request #16 from pgste/claude/feat-governed-promotion",
          "timestamp": "2026-07-09T12:57:59+01:00",
          "tree_id": "08d1505f77212c4a2cc482c05df14a364b4c5d2f",
          "url": "https://github.com/pgste/reaper/commit/cf6ed1e55fc260260991bdfd571b113cde201abe"
        },
        "date": 1783598717986,
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
            "value": 473,
            "range": "± 18",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 120,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 315,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 634,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1300,
            "range": "± 20",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}