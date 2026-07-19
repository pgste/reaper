window.BENCHMARK_DATA = {
  "lastUpdate": 1784473074401,
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
          "id": "ba498cd760c7720586f33b9d03c16a56557998b4",
          "message": "Merge pull request #88 from pgste/claude/reaper-plan06-prunability",
          "timestamp": "2026-07-19T15:53:14+01:00",
          "tree_id": "4771f9ff28d566ea74caa641830ffb89ca1abf48",
          "url": "https://github.com/pgste/reaper/commit/ba498cd760c7720586f33b9d03c16a56557998b4"
        },
        "date": 1784473073994,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 122,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 466,
            "range": "± 3",
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
            "value": 302,
            "range": "± 7",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 553,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1369,
            "range": "± 6",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}