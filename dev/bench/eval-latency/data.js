window.BENCHMARK_DATA = {
  "lastUpdate": 1784145842169,
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
          "id": "dd90260d67a95dd0cc89a7d03d118e297d0509fd",
          "message": "Merge pull request #70 from pgste/claude/reaper-f1-capability-core",
          "timestamp": "2026-07-15T20:56:03+01:00",
          "tree_id": "3c304be15cf06c0b5d4ccce45de520943aa315ea",
          "url": "https://github.com/pgste/reaper/commit/dd90260d67a95dd0cc89a7d03d118e297d0509fd"
        },
        "date": 1784145841313,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 153,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 551,
            "range": "± 32",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 130,
            "range": "± 3",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 310,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 589,
            "range": "± 7",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1283,
            "range": "± 25",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}