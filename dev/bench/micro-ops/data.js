window.BENCHMARK_DATA = {
  "lastUpdate": 1783826466141,
  "repoUrl": "https://github.com/pgste/reaper",
  "entries": {
    "Engine micro-ops (criterion)": [
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
          "id": "92f63c827845c4d49adf5acc9d45f531a0ef07c5",
          "message": "Merge pull request #33 from pgste/claude/reaper-security-perf-review-9pz54s",
          "timestamp": "2026-07-12T04:15:52+01:00",
          "tree_id": "137b8aceb34c059d4f819c37a351c32f92e8955d",
          "url": "https://github.com/pgste/reaper/commit/92f63c827845c4d49adf5acc9d45f531a0ef07c5"
        },
        "date": 1783826465794,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_hot_swap/deploy_policy",
            "value": 158324,
            "range": "± 38822",
            "unit": "ns/iter"
          },
          {
            "name": "policy_hot_swap/concurrent_lookup",
            "value": 48,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "concurrent_access/concurrent_evaluations",
            "value": 226799,
            "range": "± 2349",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/100",
            "value": 183484,
            "range": "± 323",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/1000",
            "value": 3542917,
            "range": "± 7037",
            "unit": "ns/iter"
          },
          {
            "name": "realistic_workloads/microservice_auth_pattern",
            "value": 11427,
            "range": "± 43",
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
            "value": 145,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/reachable_3hop_miss_bounded",
            "value": 293,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/inherited_1hop_hit",
            "value": 137,
            "range": "± 0",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}