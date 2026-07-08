window.BENCHMARK_DATA = {
  "lastUpdate": 1783539486582,
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
          "id": "2e5c28c25d59e9aa3adebddbe0800febad59a5a6",
          "message": "Merge pull request #9 from pgste/claude/reaper-security-perf-review-9pz54s",
          "timestamp": "2026-07-08T20:32:56+01:00",
          "tree_id": "c726a7f30abfeeba39a2c26ccef8a530ca28f619",
          "url": "https://github.com/pgste/reaper/commit/2e5c28c25d59e9aa3adebddbe0800febad59a5a6"
        },
        "date": 1783539486031,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_hot_swap/deploy_policy",
            "value": 155369,
            "range": "± 37307",
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
            "value": 375021,
            "range": "± 1827",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/100",
            "value": 148846,
            "range": "± 441",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/1000",
            "value": 3171352,
            "range": "± 3590",
            "unit": "ns/iter"
          },
          {
            "name": "realistic_workloads/microservice_auth_pattern",
            "value": 11787,
            "range": "± 15",
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
            "value": 305,
            "range": "± 6",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/inherited_1hop_hit",
            "value": 140,
            "range": "± 0",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}