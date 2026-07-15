window.BENCHMARK_DATA = {
  "lastUpdate": 1784119023386,
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
          "id": "ca4cb59dd9500a0d0a99a1c6e428792eb428ab15",
          "message": "Merge pull request #64 from pgste/claude/reaper-e1-agent-streaming-sink",
          "timestamp": "2026-07-15T13:32:23+01:00",
          "tree_id": "f8ed617a3c8a22a1570e1659c780a62b25ccd06b",
          "url": "https://github.com/pgste/reaper/commit/ca4cb59dd9500a0d0a99a1c6e428792eb428ab15"
        },
        "date": 1784119021737,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 115,
            "range": "± 4",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 369,
            "range": "± 18",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 122,
            "range": "± 3",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 280,
            "range": "± 17",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 499,
            "range": "± 14",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1072,
            "range": "± 26",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}