window.BENCHMARK_DATA = {
  "lastUpdate": 1783564416869,
  "repoUrl": "https://github.com/pgste/reaper",
  "entries": {
    "All benchmarks (criterion)": [
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
          "id": "ebeae26b9ecc9ccfc3ec0e94df1dbb372bf7a4de",
          "message": "Merge pull request #14 from pgste/claude/feat-policy-integrity",
          "timestamp": "2026-07-09T03:28:25+01:00",
          "tree_id": "7862560ff7bdb09ab13bb8471b8b711e152ec411",
          "url": "https://github.com/pgste/reaper/commit/ebeae26b9ecc9ccfc3ec0e94df1dbb372bf7a4de"
        },
        "date": 1783564416362,
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
            "value": 467,
            "range": "± 7",
            "unit": "ns/iter"
          },
          {
            "name": "policy_hot_swap/deploy_policy",
            "value": 166554,
            "range": "± 37893",
            "unit": "ns/iter"
          },
          {
            "name": "policy_hot_swap/concurrent_lookup",
            "value": 34,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "concurrent_access/concurrent_evaluations",
            "value": 309988,
            "range": "± 5615",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/100",
            "value": 150337,
            "range": "± 2311",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/1000",
            "value": 3344597,
            "range": "± 5605",
            "unit": "ns/iter"
          },
          {
            "name": "realistic_workloads/microservice_auth_pattern",
            "value": 12769,
            "range": "± 75",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 132,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/has_relation_direct_hit",
            "value": 21,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/has_relation_direct_miss",
            "value": 21,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/reachable_1hop_hit",
            "value": 196,
            "range": "± 3",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/reachable_3hop_miss_bounded",
            "value": 344,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/inherited_1hop_hit",
            "value": 151,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 357,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 688,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1414,
            "range": "± 8",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}