window.BENCHMARK_DATA = {
  "lastUpdate": 1784125327270,
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
          "id": "dcf4b113220468b65a7fc1d913379f713fed794a",
          "message": "Merge pull request #66 from pgste/claude/reaper-e4-tenant-quotas",
          "timestamp": "2026-07-15T15:17:19+01:00",
          "tree_id": "97b9467e2508772d9783a9d53757247851d6fe90",
          "url": "https://github.com/pgste/reaper/commit/dcf4b113220468b65a7fc1d913379f713fed794a"
        },
        "date": 1784125327183,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 118,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 485,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "policy_hot_swap/deploy_policy",
            "value": 153805,
            "range": "± 37256",
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
            "value": 370247,
            "range": "± 9101",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/100",
            "value": 175447,
            "range": "± 262",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/1000",
            "value": 3424144,
            "range": "± 73094",
            "unit": "ns/iter"
          },
          {
            "name": "realistic_workloads/microservice_auth_pattern",
            "value": 12094,
            "range": "± 32",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 118,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/has_relation_direct_hit",
            "value": 22,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/has_relation_direct_miss",
            "value": 22,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/reachable_1hop_hit",
            "value": 66,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/reachable_3hop_miss_bounded",
            "value": 66,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/inherited_1hop_hit",
            "value": 24,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 301,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 553,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1278,
            "range": "± 10",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}