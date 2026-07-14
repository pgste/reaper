window.BENCHMARK_DATA = {
  "lastUpdate": 1783997752167,
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
          "id": "420171a6622c991350211b0759b368bd81739264",
          "message": "Merge pull request #59 from pgste/claude/reaper-security-perf-review-9pz54s",
          "timestamp": "2026-07-14T03:51:01+01:00",
          "tree_id": "a2c5e7be6bebdc735a77878d9cf3c3ed79b47970",
          "url": "https://github.com/pgste/reaper/commit/420171a6622c991350211b0759b368bd81739264"
        },
        "date": 1783997752024,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 131,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 451,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "policy_hot_swap/deploy_policy",
            "value": 180600,
            "range": "± 36868",
            "unit": "ns/iter"
          },
          {
            "name": "policy_hot_swap/concurrent_lookup",
            "value": 34,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "concurrent_access/concurrent_evaluations",
            "value": 304250,
            "range": "± 3505",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/100",
            "value": 184071,
            "range": "± 532",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/1000",
            "value": 3863683,
            "range": "± 6901",
            "unit": "ns/iter"
          },
          {
            "name": "realistic_workloads/microservice_auth_pattern",
            "value": 12828,
            "range": "± 26",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 131,
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
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/reachable_1hop_hit",
            "value": 57,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/reachable_3hop_miss_bounded",
            "value": 57,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/inherited_1hop_hit",
            "value": 23,
            "range": "± 0",
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
            "value": 594,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1294,
            "range": "± 21",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}