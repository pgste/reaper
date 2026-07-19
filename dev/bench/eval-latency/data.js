window.BENCHMARK_DATA = {
  "lastUpdate": 1784489962622,
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
          "id": "abd233dcff60052c408f25dcb4b6ed75761ec567",
          "message": "Merge pull request #90 from pgste/claude/reaper-ci-bdd-gate",
          "timestamp": "2026-07-19T20:31:23+01:00",
          "tree_id": "9153198b9efc483ee871eea0a8c90b4b2d95af05",
          "url": "https://github.com/pgste/reaper/commit/abd233dcff60052c408f25dcb4b6ed75761ec567"
        },
        "date": 1784489888165,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 109,
            "range": "± 5",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 331,
            "range": "± 8",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 109,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 246,
            "range": "± 5",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 440,
            "range": "± 11",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 976,
            "range": "± 19",
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
          "id": "b68d1f9274a234b02fa2afd9343492b68fdcb71a",
          "message": "Merge pull request #91 from pgste/claude/reaper-slo-abac-row",
          "timestamp": "2026-07-19T20:31:41+01:00",
          "tree_id": "231d7d93e26d79b918e69745c330d0f80c5a8aab",
          "url": "https://github.com/pgste/reaper/commit/b68d1f9274a234b02fa2afd9343492b68fdcb71a"
        },
        "date": 1784489933641,
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
            "range": "± 0",
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
            "value": 303,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 555,
            "range": "± 4",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1319,
            "range": "± 12",
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
          "id": "f20d50ff4686b8daf0412962654e019c4db3c112",
          "message": "Merge pull request #92 from pgste/claude/reaper-partial-eval-tier1",
          "timestamp": "2026-07-19T20:32:13+01:00",
          "tree_id": "fc97aa866383d419f5c9a51720e9a9ec736b0b54",
          "url": "https://github.com/pgste/reaper/commit/f20d50ff4686b8daf0412962654e019c4db3c112"
        },
        "date": 1784489961363,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 134,
            "range": "± 3",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 449,
            "range": "± 13",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 125,
            "range": "± 5",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 303,
            "range": "± 3",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 559,
            "range": "± 12",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1338,
            "range": "± 13",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}