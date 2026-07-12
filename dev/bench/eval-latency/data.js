window.BENCHMARK_DATA = {
  "lastUpdate": 1783874098932,
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
          "id": "932ba180a316aae059bd362582d9635528aa948d",
          "message": "Merge pull request #38 from pgste/claude/reaper-security-perf-review-9pz54s",
          "timestamp": "2026-07-12T17:30:10+01:00",
          "tree_id": "57fa95735a352b5945ae574db1d0d60c0082ad6c",
          "url": "https://github.com/pgste/reaper/commit/932ba180a316aae059bd362582d9635528aa948d"
        },
        "date": 1783874097729,
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
            "value": 497,
            "range": "± 3",
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
            "value": 305,
            "range": "± 0",
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
            "range": "± 7",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}