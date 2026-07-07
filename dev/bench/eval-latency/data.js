window.BENCHMARK_DATA = {
  "lastUpdate": 1783451841548,
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
          "id": "aad4e4ffbd5096e332dcd343cf80aafed689682b",
          "message": "Merge pull request #7 from pgste/claude/reaper-security-perf-review-9pz54s",
          "timestamp": "2026-07-07T20:10:13+01:00",
          "tree_id": "2e9edfd88e2c755c27f74b39b9d00306013dca67",
          "url": "https://github.com/pgste/reaper/commit/aad4e4ffbd5096e332dcd343cf80aafed689682b"
        },
        "date": 1783451840146,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 110,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 359,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 110,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 313,
            "range": "± 5",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 641,
            "range": "± 5",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1166,
            "range": "± 7",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}