window.BENCHMARK_DATA = {
  "lastUpdate": 1784078985944,
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
          "id": "13623b4644645146e209be4d85469f558b3d43d3",
          "message": "Merge pull request #62 from pgste/claude/reaper-e2-erasure-followups-cq3c2o",
          "timestamp": "2026-07-15T02:23:41+01:00",
          "tree_id": "736eac55da8a684835a2703c29b9e902f37083b4",
          "url": "https://github.com/pgste/reaper/commit/13623b4644645146e209be4d85469f558b3d43d3"
        },
        "date": 1784078985862,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_hot_swap/deploy_policy",
            "value": 136278,
            "range": "± 37647",
            "unit": "ns/iter"
          },
          {
            "name": "policy_hot_swap/concurrent_lookup",
            "value": 37,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "concurrent_access/concurrent_evaluations",
            "value": 193152,
            "range": "± 1432",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/100",
            "value": 130792,
            "range": "± 6676",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/1000",
            "value": 2671019,
            "range": "± 160254",
            "unit": "ns/iter"
          },
          {
            "name": "realistic_workloads/microservice_auth_pattern",
            "value": 8968,
            "range": "± 102",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/has_relation_direct_hit",
            "value": 17,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/has_relation_direct_miss",
            "value": 18,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/reachable_1hop_hit",
            "value": 39,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/reachable_3hop_miss_bounded",
            "value": 39,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/inherited_1hop_hit",
            "value": 17,
            "range": "± 0",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}