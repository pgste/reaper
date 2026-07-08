window.BENCHMARK_DATA = {
  "lastUpdate": 1783547584287,
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
          "id": "77a284475451ac39a803a2638cb24c22437545c2",
          "message": "Merge pull request #12 from pgste/claude/feat-policy-integrity",
          "timestamp": "2026-07-08T22:48:10+01:00",
          "tree_id": "08c244c4100bac6bac6d5487819b7b374d6791fe",
          "url": "https://github.com/pgste/reaper/commit/77a284475451ac39a803a2638cb24c22437545c2"
        },
        "date": 1783547584168,
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
            "value": 475,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "policy_hot_swap/deploy_policy",
            "value": 158442,
            "range": "± 36576",
            "unit": "ns/iter"
          },
          {
            "name": "policy_hot_swap/concurrent_lookup",
            "value": 33,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "concurrent_access/concurrent_evaluations",
            "value": 371519,
            "range": "± 2909",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/100",
            "value": 147252,
            "range": "± 252",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/1000",
            "value": 3194539,
            "range": "± 6036",
            "unit": "ns/iter"
          },
          {
            "name": "realistic_workloads/microservice_auth_pattern",
            "value": 11986,
            "range": "± 37",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 121,
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
            "value": 157,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/reachable_3hop_miss_bounded",
            "value": 307,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/inherited_1hop_hit",
            "value": 141,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 327,
            "range": "± 6",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 637,
            "range": "± 3",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1284,
            "range": "± 15",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}