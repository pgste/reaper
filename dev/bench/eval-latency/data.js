window.BENCHMARK_DATA = {
  "lastUpdate": 1783622843721,
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
          "id": "214b483d8457d5b1f29325e1b33294ec2c30f3fc",
          "message": "Merge pull request #17 from pgste/claude/feat-enterprise-identity",
          "timestamp": "2026-07-09T19:40:29+01:00",
          "tree_id": "1fd9b7b199b4736927bcc4fde28966dbcdcaf681",
          "url": "https://github.com/pgste/reaper/commit/214b483d8457d5b1f29325e1b33294ec2c30f3fc"
        },
        "date": 1783622842260,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 109,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 357,
            "range": "± 11",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 110,
            "range": "± 5",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 269,
            "range": "± 14",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 551,
            "range": "± 20",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1030,
            "range": "± 32",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}