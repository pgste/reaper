window.BENCHMARK_DATA = {
  "lastUpdate": 1783451858072,
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
          "id": "aad4e4ffbd5096e332dcd343cf80aafed689682b",
          "message": "Merge pull request #7 from pgste/claude/reaper-security-perf-review-9pz54s",
          "timestamp": "2026-07-07T20:10:13+01:00",
          "tree_id": "2e9edfd88e2c755c27f74b39b9d00306013dca67",
          "url": "https://github.com/pgste/reaper/commit/aad4e4ffbd5096e332dcd343cf80aafed689682b"
        },
        "date": 1783451857605,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_hot_swap/deploy_policy",
            "value": 155136,
            "range": "± 37749",
            "unit": "ns/iter"
          },
          {
            "name": "policy_hot_swap/concurrent_lookup",
            "value": 49,
            "range": "± 4",
            "unit": "ns/iter"
          },
          {
            "name": "concurrent_access/concurrent_evaluations",
            "value": 229557,
            "range": "± 3157",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/100",
            "value": 157804,
            "range": "± 332",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/1000",
            "value": 3281963,
            "range": "± 4771",
            "unit": "ns/iter"
          },
          {
            "name": "realistic_workloads/microservice_auth_pattern",
            "value": 11649,
            "range": "± 164",
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
            "value": 141,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/reachable_3hop_miss_bounded",
            "value": 286,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/inherited_1hop_hit",
            "value": 133,
            "range": "± 0",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}