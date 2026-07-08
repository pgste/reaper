window.BENCHMARK_DATA = {
  "lastUpdate": 1783544227384,
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
          "id": "18d205f9d45608a789af5806542451f54d261080",
          "message": "Merge pull request #11 from pgste/claude/feat-authn-authz-foundation",
          "timestamp": "2026-07-08T21:52:25+01:00",
          "tree_id": "2f3d0b83b3a59e8a9ed5a52d2f9f57e20b3e41a0",
          "url": "https://github.com/pgste/reaper/commit/18d205f9d45608a789af5806542451f54d261080"
        },
        "date": 1783544226993,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_hot_swap/deploy_policy",
            "value": 135922,
            "range": "± 37542",
            "unit": "ns/iter"
          },
          {
            "name": "policy_hot_swap/concurrent_lookup",
            "value": 25,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "concurrent_access/concurrent_evaluations",
            "value": 234637,
            "range": "± 1503",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/100",
            "value": 118118,
            "range": "± 130",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/1000",
            "value": 2619142,
            "range": "± 4637",
            "unit": "ns/iter"
          },
          {
            "name": "realistic_workloads/microservice_auth_pattern",
            "value": 9902,
            "range": "± 22",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/has_relation_direct_hit",
            "value": 16,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/has_relation_direct_miss",
            "value": 16,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/reachable_1hop_hit",
            "value": 153,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/reachable_3hop_miss_bounded",
            "value": 263,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/inherited_1hop_hit",
            "value": 116,
            "range": "± 1",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}