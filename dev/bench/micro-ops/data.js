window.BENCHMARK_DATA = {
  "lastUpdate": 1784121018911,
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
          "id": "256e3444aad6ba160bfba7fc814b029acc2d3b01",
          "message": "Merge pull request #65 from pgste/claude/reaper-e3-airgap-signing",
          "timestamp": "2026-07-15T14:05:24+01:00",
          "tree_id": "4bc75b78916779bb7342f7e67b1730f6d1cff05f",
          "url": "https://github.com/pgste/reaper/commit/256e3444aad6ba160bfba7fc814b029acc2d3b01"
        },
        "date": 1784121018760,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_hot_swap/deploy_policy",
            "value": 167753,
            "range": "± 37118",
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
            "value": 298337,
            "range": "± 2848",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/100",
            "value": 178222,
            "range": "± 220",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/1000",
            "value": 3627541,
            "range": "± 6475",
            "unit": "ns/iter"
          },
          {
            "name": "realistic_workloads/microservice_auth_pattern",
            "value": 12837,
            "range": "± 131",
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
            "range": "± 1",
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
            "value": 25,
            "range": "± 0",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}