window.BENCHMARK_DATA = {
  "lastUpdate": 1784234461539,
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
          "id": "7ebd4450078c231f63748dc1c0b9023d044d083f",
          "message": "Merge pull request #80 from pgste/claude/reaper-plan03-decision-rollback",
          "timestamp": "2026-07-16T21:33:49+01:00",
          "tree_id": "e2ee59f867219266876faf53d740f6c9bb2dd431",
          "url": "https://github.com/pgste/reaper/commit/7ebd4450078c231f63748dc1c0b9023d044d083f"
        },
        "date": 1784234460116,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 121,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 443,
            "range": "± 21",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 122,
            "range": "± 5",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 301,
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
            "value": 1288,
            "range": "± 11",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}