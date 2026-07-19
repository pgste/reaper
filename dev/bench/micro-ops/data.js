window.BENCHMARK_DATA = {
  "lastUpdate": 1784458490547,
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
          "id": "ee841db3c3adf9c6e0bf7fcc38a7372a16244631",
          "message": "Merge pull request #87 from pgste/claude/reaper-plan06-pagination-b3",
          "timestamp": "2026-07-19T11:47:40+01:00",
          "tree_id": "3ca81e93eac84c8234348fa2c6f593e8a2cd22e7",
          "url": "https://github.com/pgste/reaper/commit/ee841db3c3adf9c6e0bf7fcc38a7372a16244631"
        },
        "date": 1784458490401,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_hot_swap/deploy_policy",
            "value": 149021,
            "range": "± 37264",
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
            "value": 405902,
            "range": "± 2217",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/100",
            "value": 175260,
            "range": "± 306",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/1000",
            "value": 3373900,
            "range": "± 6453",
            "unit": "ns/iter"
          },
          {
            "name": "realistic_workloads/microservice_auth_pattern",
            "value": 11911,
            "range": "± 20",
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
            "value": 66,
            "range": "± 0",
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
          }
        ]
      }
    ]
  }
}