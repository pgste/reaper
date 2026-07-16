window.BENCHMARK_DATA = {
  "lastUpdate": 1784220248986,
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
          "id": "c3de74a5a896d28b6a40a3550115dd51402a51ca",
          "message": "Merge pull request #77 from pgste/claude/reaper-enterprise-review-mlwzsk",
          "timestamp": "2026-07-16T17:37:02+01:00",
          "tree_id": "c048310e351f3647b7db2f5edc36f082fab46348",
          "url": "https://github.com/pgste/reaper/commit/c3de74a5a896d28b6a40a3550115dd51402a51ca"
        },
        "date": 1784220247810,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 121,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 445,
            "range": "± 5",
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
            "value": 305,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 562,
            "range": "± 9",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1339,
            "range": "± 5",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}