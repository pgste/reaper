window.BENCHMARK_DATA = {
  "lastUpdate": 1784287957074,
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
          "id": "1ef094726dfe04f37d517803b05809a82ef74322",
          "message": "Merge pull request #83 from pgste/claude/reaper-plan05-perf-and-followups",
          "timestamp": "2026-07-17T12:27:44+01:00",
          "tree_id": "5bc634c4638f1baedf197c70153372cb18a6aaf6",
          "url": "https://github.com/pgste/reaper/commit/1ef094726dfe04f37d517803b05809a82ef74322"
        },
        "date": 1784287956576,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 134,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 470,
            "range": "± 6",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 134,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 334,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 611,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1318,
            "range": "± 11",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}