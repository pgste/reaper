window.BENCHMARK_DATA = {
  "lastUpdate": 1784060066689,
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
          "id": "10de38acdd36e8fdd40c1aeca3f49aa3fd5d913b",
          "message": "Merge pull request #61 from pgste/claude/reaper-security-perf-review-9pz54s",
          "timestamp": "2026-07-14T21:09:41+01:00",
          "tree_id": "e116b05a194bfd4848d696f42a7420f4e011cb94",
          "url": "https://github.com/pgste/reaper/commit/10de38acdd36e8fdd40c1aeca3f49aa3fd5d913b"
        },
        "date": 1784060066171,
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
            "value": 457,
            "range": "± 9",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 120,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 302,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 557,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1286,
            "range": "± 36",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}