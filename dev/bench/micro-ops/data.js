window.BENCHMARK_DATA = {
  "lastUpdate": 1783906733499,
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
          "id": "fc9442692f1ff9ecedbd64a1391141be347ae107",
          "message": "Merge pull request #46 from pgste/claude/reaper-security-perf-review-9pz54s",
          "timestamp": "2026-07-13T02:33:46+01:00",
          "tree_id": "de7d6a48a8c076672f2e7d7ec016e8e2deae8332",
          "url": "https://github.com/pgste/reaper/commit/fc9442692f1ff9ecedbd64a1391141be347ae107"
        },
        "date": 1783906733030,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_hot_swap/deploy_policy",
            "value": 148343,
            "range": "± 36721",
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
            "value": 363692,
            "range": "± 3687",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/100",
            "value": 170060,
            "range": "± 289",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/1000",
            "value": 3320101,
            "range": "± 4028",
            "unit": "ns/iter"
          },
          {
            "name": "realistic_workloads/microservice_auth_pattern",
            "value": 12029,
            "range": "± 52",
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
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/reachable_3hop_miss_bounded",
            "value": 65,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/inherited_1hop_hit",
            "value": 28,
            "range": "± 0",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}