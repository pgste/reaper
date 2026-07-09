window.BENCHMARK_DATA = {
  "lastUpdate": 1783622860199,
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
          "id": "214b483d8457d5b1f29325e1b33294ec2c30f3fc",
          "message": "Merge pull request #17 from pgste/claude/feat-enterprise-identity",
          "timestamp": "2026-07-09T19:40:29+01:00",
          "tree_id": "1fd9b7b199b4736927bcc4fde28966dbcdcaf681",
          "url": "https://github.com/pgste/reaper/commit/214b483d8457d5b1f29325e1b33294ec2c30f3fc"
        },
        "date": 1783622859802,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_hot_swap/deploy_policy",
            "value": 153983,
            "range": "± 37950",
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
            "value": 194275,
            "range": "± 1642",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/100",
            "value": 139524,
            "range": "± 1001",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/1000",
            "value": 3070379,
            "range": "± 21612",
            "unit": "ns/iter"
          },
          {
            "name": "realistic_workloads/microservice_auth_pattern",
            "value": 11036,
            "range": "± 516",
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
            "value": 118,
            "range": "± 4",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/reachable_3hop_miss_bounded",
            "value": 236,
            "range": "± 10",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/inherited_1hop_hit",
            "value": 113,
            "range": "± 6",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}