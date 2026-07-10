window.BENCHMARK_DATA = {
  "lastUpdate": 1783685665605,
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
          "id": "d65ccc3a9da534d62ba18d2f2a0da4e6ae285940",
          "message": "Merge pull request #23 from pgste/claude/feat-replay-engine",
          "timestamp": "2026-07-10T13:09:34+01:00",
          "tree_id": "f982730121a208b358b8f4e20667045ef68b1aee",
          "url": "https://github.com/pgste/reaper/commit/d65ccc3a9da534d62ba18d2f2a0da4e6ae285940"
        },
        "date": 1783685664093,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 120,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 456,
            "range": "± 4",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 119,
            "range": "± 5",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 319,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 646,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1344,
            "range": "± 13",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}