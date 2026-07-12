window.BENCHMARK_DATA = {
  "lastUpdate": 1783893277634,
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
          "id": "ca6998ed97acbccdef87248e1d91eb8c5dbd7133",
          "message": "Merge pull request #41 from pgste/claude/reaper-security-perf-review-9pz54s",
          "timestamp": "2026-07-12T22:49:58+01:00",
          "tree_id": "aaea1f00c9bf2e815405d664f2d19272a2175b72",
          "url": "https://github.com/pgste/reaper/commit/ca6998ed97acbccdef87248e1d91eb8c5dbd7133"
        },
        "date": 1783893277126,
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
            "value": 475,
            "range": "± 5",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 119,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 306,
            "range": "± 10",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 550,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1293,
            "range": "± 8",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}