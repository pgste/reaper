window.BENCHMARK_DATA = {
  "lastUpdate": 1783870838278,
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
          "id": "ffade2133ab317ac98ae15d63319497791493c3a",
          "message": "Merge pull request #37 from pgste/claude/reaper-security-perf-review-9pz54s",
          "timestamp": "2026-07-12T16:35:52+01:00",
          "tree_id": "9ebb932f8c81d56e18710370502fca9057190af7",
          "url": "https://github.com/pgste/reaper/commit/ffade2133ab317ac98ae15d63319497791493c3a"
        },
        "date": 1783870837018,
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
            "value": 474,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 118,
            "range": "± 3",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 302,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 555,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1314,
            "range": "± 11",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}