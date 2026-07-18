window.BENCHMARK_DATA = {
  "lastUpdate": 1784410247143,
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
          "id": "2fba0ba5e251406a10658ea5b1c2895ff1461ebd",
          "message": "Merge pull request #84 from pgste/claude/reaper-coverage-gate",
          "timestamp": "2026-07-17T14:26:51+01:00",
          "tree_id": "97f110fded1a61ee7738e4a2928fd7f5ae2bbcfd",
          "url": "https://github.com/pgste/reaper/commit/2fba0ba5e251406a10658ea5b1c2895ff1461ebd"
        },
        "date": 1784295239633,
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
            "value": 447,
            "range": "± 11",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 121,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 307,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 564,
            "range": "± 3",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1347,
            "range": "± 20",
            "unit": "ns/iter"
          }
        ]
      },
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
          "id": "94d3bc8e099d7046210f2a6e53d7cf8584ceab79",
          "message": "Merge pull request #85 from pgste/claude/reaper-plan06-ga-hardening",
          "timestamp": "2026-07-18T22:25:55+01:00",
          "tree_id": "729dc9379a34117f573cfb0c82c79d7b50d924e4",
          "url": "https://github.com/pgste/reaper/commit/94d3bc8e099d7046210f2a6e53d7cf8584ceab79"
        },
        "date": 1784410245523,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 121,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 440,
            "range": "± 2",
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
            "range": "± 4",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 555,
            "range": "± 5",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1313,
            "range": "± 10",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}