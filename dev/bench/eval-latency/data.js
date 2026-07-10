window.BENCHMARK_DATA = {
  "lastUpdate": 1783722105903,
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
          "id": "e0879a78a839276473a2aa67f018ef5ed9b89dcf",
          "message": "Merge pull request #25 from pgste/claude/feat-software-supply-chain",
          "timestamp": "2026-07-10T23:14:31+01:00",
          "tree_id": "369a8c6da8e216ca21111160b461c98693075d8b",
          "url": "https://github.com/pgste/reaper/commit/e0879a78a839276473a2aa67f018ef5ed9b89dcf"
        },
        "date": 1783722104388,
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
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 122,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 318,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 633,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1300,
            "range": "± 15",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}