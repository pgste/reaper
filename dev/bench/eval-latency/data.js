window.BENCHMARK_DATA = {
  "lastUpdate": 1783906716242,
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
          "id": "fc9442692f1ff9ecedbd64a1391141be347ae107",
          "message": "Merge pull request #46 from pgste/claude/reaper-security-perf-review-9pz54s",
          "timestamp": "2026-07-13T02:33:46+01:00",
          "tree_id": "de7d6a48a8c076672f2e7d7ec016e8e2deae8332",
          "url": "https://github.com/pgste/reaper/commit/fc9442692f1ff9ecedbd64a1391141be347ae107"
        },
        "date": 1783906714997,
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
            "value": 475,
            "range": "± 14",
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
            "value": 300,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 552,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1312,
            "range": "± 13",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}