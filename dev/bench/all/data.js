window.BENCHMARK_DATA = {
  "lastUpdate": 1783732445405,
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
          "id": "342d0d022c2fd83750182bf0c6c258eb567d9c6f",
          "message": "Merge pull request #26 from pgste/claude/deps-latest-upgrades",
          "timestamp": "2026-07-11T02:06:20+01:00",
          "tree_id": "3d9b18563acdf866cb4e3390353997e9cef3bf69",
          "url": "https://github.com/pgste/reaper/commit/342d0d022c2fd83750182bf0c6c258eb567d9c6f"
        },
        "date": 1783732443966,
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
            "value": 471,
            "range": "± 6",
            "unit": "ns/iter"
          },
          {
            "name": "policy_hot_swap/deploy_policy",
            "value": 149465,
            "range": "± 37675",
            "unit": "ns/iter"
          },
          {
            "name": "policy_hot_swap/concurrent_lookup",
            "value": 32,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "concurrent_access/concurrent_evaluations",
            "value": 351319,
            "range": "± 10284",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/100",
            "value": 147982,
            "range": "± 401",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/1000",
            "value": 3100492,
            "range": "± 11154",
            "unit": "ns/iter"
          },
          {
            "name": "realistic_workloads/microservice_auth_pattern",
            "value": 12060,
            "range": "± 374",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 119,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/has_relation_direct_hit",
            "value": 23,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/has_relation_direct_miss",
            "value": 23,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/reachable_1hop_hit",
            "value": 156,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/reachable_3hop_miss_bounded",
            "value": 306,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/inherited_1hop_hit",
            "value": 140,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 321,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 634,
            "range": "± 21",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1289,
            "range": "± 7",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}