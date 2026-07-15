window.BENCHMARK_DATA = {
  "lastUpdate": 1784139808422,
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
          "id": "195895734ee71266409272d0ff29ed3122fd4507",
          "message": "Merge pull request #69 from pgste/claude/reaper-f2-wasm-slice3",
          "timestamp": "2026-07-15T19:15:57+01:00",
          "tree_id": "880c4a54c40ee78d0e501e3d304d1b2074f71cd7",
          "url": "https://github.com/pgste/reaper/commit/195895734ee71266409272d0ff29ed3122fd4507"
        },
        "date": 1784139807582,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 131,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 474,
            "range": "± 5",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 130,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 311,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 600,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1345,
            "range": "± 22",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}