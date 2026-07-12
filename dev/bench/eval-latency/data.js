window.BENCHMARK_DATA = {
  "lastUpdate": 1783885391250,
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
          "id": "fb8afe927917aed04e32c3799e0509ab1da2ac57",
          "message": "Merge pull request #39 from pgste/claude/reaper-security-perf-review-9pz54s",
          "timestamp": "2026-07-12T20:38:52+01:00",
          "tree_id": "709160a58a5519ad00ee0259600a0b063841b689",
          "url": "https://github.com/pgste/reaper/commit/fb8afe927917aed04e32c3799e0509ab1da2ac57"
        },
        "date": 1783885389543,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 92,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 229,
            "range": "± 5",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 90,
            "range": "± 5",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 198,
            "range": "± 8",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 362,
            "range": "± 5",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 742,
            "range": "± 26",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}