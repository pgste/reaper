window.BENCHMARK_DATA = {
  "lastUpdate": 1784164047426,
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
          "id": "c7b4cbf44f0adae113fb400000d7ec5814a75787",
          "message": "Merge pull request #73 from pgste/claude/reaper-f1-agent-capability",
          "timestamp": "2026-07-16T02:02:57+01:00",
          "tree_id": "ea4560b3ab8db0eddfac80448805dcbc50c25d4d",
          "url": "https://github.com/pgste/reaper/commit/c7b4cbf44f0adae113fb400000d7ec5814a75787"
        },
        "date": 1784164047327,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_hot_swap/deploy_policy",
            "value": 154839,
            "range": "± 38124",
            "unit": "ns/iter"
          },
          {
            "name": "policy_hot_swap/concurrent_lookup",
            "value": 44,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "concurrent_access/concurrent_evaluations",
            "value": 206754,
            "range": "± 1712",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/100",
            "value": 158621,
            "range": "± 283",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/1000",
            "value": 3203162,
            "range": "± 37576",
            "unit": "ns/iter"
          },
          {
            "name": "realistic_workloads/microservice_auth_pattern",
            "value": 10827,
            "range": "± 59",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/has_relation_direct_hit",
            "value": 20,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/has_relation_direct_miss",
            "value": 20,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/reachable_1hop_hit",
            "value": 50,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/reachable_3hop_miss_bounded",
            "value": 50,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/inherited_1hop_hit",
            "value": 22,
            "range": "± 0",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}