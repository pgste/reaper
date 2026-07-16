window.BENCHMARK_DATA = {
  "lastUpdate": 1784164045880,
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
          "id": "c7b4cbf44f0adae113fb400000d7ec5814a75787",
          "message": "Merge pull request #73 from pgste/claude/reaper-f1-agent-capability",
          "timestamp": "2026-07-16T02:02:57+01:00",
          "tree_id": "ea4560b3ab8db0eddfac80448805dcbc50c25d4d",
          "url": "https://github.com/pgste/reaper/commit/c7b4cbf44f0adae113fb400000d7ec5814a75787"
        },
        "date": 1784164044407,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 106,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 341,
            "range": "± 3",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 106,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 249,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 440,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 971,
            "range": "± 3",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}