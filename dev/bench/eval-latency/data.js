window.BENCHMARK_DATA = {
  "lastUpdate": 1783982040265,
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
          "id": "59796871a9df1b5273083d27f69c42f59316adfb",
          "message": "Merge pull request #57 from pgste/claude/reaper-security-perf-review-9pz54s",
          "timestamp": "2026-07-13T23:29:07+01:00",
          "tree_id": "f79bf9b331bf0816690aadebdeadca8350e492e2",
          "url": "https://github.com/pgste/reaper/commit/59796871a9df1b5273083d27f69c42f59316adfb"
        },
        "date": 1783982039723,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 130,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 475,
            "range": "± 7",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 131,
            "range": "± 3",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 316,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 600,
            "range": "± 4",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1292,
            "range": "± 28",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}