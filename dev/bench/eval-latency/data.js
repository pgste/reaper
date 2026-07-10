window.BENCHMARK_DATA = {
  "lastUpdate": 1783647158497,
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
          "id": "b17325d53294b5612fb3f43c08cbadd6c040c22f",
          "message": "Merge pull request #22 from pgste/claude/feat-replay-capture",
          "timestamp": "2026-07-10T02:27:51+01:00",
          "tree_id": "e97613ec24a1cf6b6ac32d1b91cd32be6b7b1f5b",
          "url": "https://github.com/pgste/reaper/commit/b17325d53294b5612fb3f43c08cbadd6c040c22f"
        },
        "date": 1783647157292,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 118,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 472,
            "range": "± 1",
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
            "value": 319,
            "range": "± 6",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 646,
            "range": "± 4",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1294,
            "range": "± 16",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}