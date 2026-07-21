window.BENCHMARK_DATA = {
  "lastUpdate": 1784612342133,
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
          "id": "4f8610d2a56d3f11372b329de33a1ecd96d77989",
          "message": "Merge pull request #97 from pgste/claude/reaper-slo-agentic",
          "timestamp": "2026-07-20T10:45:50+01:00",
          "tree_id": "8e9c1a06dd62c5c9838888168cd59c0c21f1aca0",
          "url": "https://github.com/pgste/reaper/commit/4f8610d2a56d3f11372b329de33a1ecd96d77989"
        },
        "date": 1784541124041,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_hot_swap/deploy_policy",
            "value": 138780,
            "range": "± 39119",
            "unit": "ns/iter"
          },
          {
            "name": "policy_hot_swap/concurrent_lookup",
            "value": 37,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "concurrent_access/concurrent_evaluations",
            "value": 182728,
            "range": "± 7136",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/100",
            "value": 131979,
            "range": "± 247",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/1000",
            "value": 2670763,
            "range": "± 9332",
            "unit": "ns/iter"
          },
          {
            "name": "realistic_workloads/microservice_auth_pattern",
            "value": 9511,
            "range": "± 461",
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
            "value": 39,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/reachable_3hop_miss_bounded",
            "value": 39,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/inherited_1hop_hit",
            "value": 17,
            "range": "± 0",
            "unit": "ns/iter"
          }
        ]
      },
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
          "id": "32163ec34da3657ca88bf1e5adc8a56bf40feb8d",
          "message": "Merge pull request #96 from pgste/claude/reaper-plan06-phase-d",
          "timestamp": "2026-07-20T10:45:34+01:00",
          "tree_id": "c14a4ccedfaa0d766ee44b60e738a572e5964e13",
          "url": "https://github.com/pgste/reaper/commit/32163ec34da3657ca88bf1e5adc8a56bf40feb8d"
        },
        "date": 1784541161067,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_hot_swap/deploy_policy",
            "value": 154107,
            "range": "± 36768",
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
            "value": 408114,
            "range": "± 9019",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/100",
            "value": 175353,
            "range": "± 358",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/1000",
            "value": 3383442,
            "range": "± 6660",
            "unit": "ns/iter"
          },
          {
            "name": "realistic_workloads/microservice_auth_pattern",
            "value": 12244,
            "range": "± 64",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/has_relation_direct_hit",
            "value": 22,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/has_relation_direct_miss",
            "value": 22,
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
      },
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
          "id": "07b53e42ffe20342d90661ccf1530e160a1b8bc6",
          "message": "Merge pull request #98 from pgste/claude/reaper-plan06-phase-e",
          "timestamp": "2026-07-20T13:43:17+01:00",
          "tree_id": "e5afcb8162441134350a693849b164fa818e5306",
          "url": "https://github.com/pgste/reaper/commit/07b53e42ffe20342d90661ccf1530e160a1b8bc6"
        },
        "date": 1784551692310,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_hot_swap/deploy_policy",
            "value": 180560,
            "range": "± 37375",
            "unit": "ns/iter"
          },
          {
            "name": "policy_hot_swap/concurrent_lookup",
            "value": 50,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "concurrent_access/concurrent_evaluations",
            "value": 214437,
            "range": "± 1733",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/100",
            "value": 185861,
            "range": "± 614",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/1000",
            "value": 3734360,
            "range": "± 4616",
            "unit": "ns/iter"
          },
          {
            "name": "realistic_workloads/microservice_auth_pattern",
            "value": 12695,
            "range": "± 49",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/has_relation_direct_hit",
            "value": 24,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/has_relation_direct_miss",
            "value": 24,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/reachable_1hop_hit",
            "value": 59,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/reachable_3hop_miss_bounded",
            "value": 59,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/inherited_1hop_hit",
            "value": 26,
            "range": "± 0",
            "unit": "ns/iter"
          }
        ]
      },
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
          "id": "eb7ca33db10d083443961d4df95b3245fe721fa8",
          "message": "Merge pull request #99 from pgste/claude/reaper-plan06-phase-f",
          "timestamp": "2026-07-20T22:34:34+01:00",
          "tree_id": "d9202b5b93330e980b70638c0896b478240f9852",
          "url": "https://github.com/pgste/reaper/commit/eb7ca33db10d083443961d4df95b3245fe721fa8"
        },
        "date": 1784583578055,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_hot_swap/deploy_policy",
            "value": 182027,
            "range": "± 37930",
            "unit": "ns/iter"
          },
          {
            "name": "policy_hot_swap/concurrent_lookup",
            "value": 50,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "concurrent_access/concurrent_evaluations",
            "value": 226754,
            "range": "± 3517",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/100",
            "value": 186651,
            "range": "± 335",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/1000",
            "value": 3751116,
            "range": "± 9155",
            "unit": "ns/iter"
          },
          {
            "name": "realistic_workloads/microservice_auth_pattern",
            "value": 12837,
            "range": "± 97",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/has_relation_direct_hit",
            "value": 24,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/has_relation_direct_miss",
            "value": 24,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/reachable_1hop_hit",
            "value": 58,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/reachable_3hop_miss_bounded",
            "value": 58,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/inherited_1hop_hit",
            "value": 26,
            "range": "± 0",
            "unit": "ns/iter"
          }
        ]
      },
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
          "id": "65454dbbf8015969f0e18ae94b5602d7a159c39b",
          "message": "Merge pull request #100 from pgste/claude/reaper-filter-design",
          "timestamp": "2026-07-20T22:34:53+01:00",
          "tree_id": "53dea1bcec972cf2ac60908afa7e8b9488c7408b",
          "url": "https://github.com/pgste/reaper/commit/65454dbbf8015969f0e18ae94b5602d7a159c39b"
        },
        "date": 1784583589117,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_hot_swap/deploy_policy",
            "value": 157861,
            "range": "± 37178",
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
            "value": 394369,
            "range": "± 8084",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/100",
            "value": 172595,
            "range": "± 606",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/1000",
            "value": 3379402,
            "range": "± 38562",
            "unit": "ns/iter"
          },
          {
            "name": "realistic_workloads/microservice_auth_pattern",
            "value": 12249,
            "range": "± 73",
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
      },
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
          "id": "4d75f3a0dff6d51b4e78a1a351e464ca8015fb53",
          "message": "Merge pull request #101 from pgste/claude/reaper-rego-gap-analysis",
          "timestamp": "2026-07-20T22:36:23+01:00",
          "tree_id": "e51222878fddb2e372fb884935bcf418579c62d0",
          "url": "https://github.com/pgste/reaper/commit/4d75f3a0dff6d51b4e78a1a351e464ca8015fb53"
        },
        "date": 1784583662890,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_hot_swap/deploy_policy",
            "value": 161274,
            "range": "± 37885",
            "unit": "ns/iter"
          },
          {
            "name": "policy_hot_swap/concurrent_lookup",
            "value": 43,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "concurrent_access/concurrent_evaluations",
            "value": 196339,
            "range": "± 2586",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/100",
            "value": 168669,
            "range": "± 486",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/1000",
            "value": 3298444,
            "range": "± 3773",
            "unit": "ns/iter"
          },
          {
            "name": "realistic_workloads/microservice_auth_pattern",
            "value": 10961,
            "range": "± 32",
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
            "value": 23,
            "range": "± 0",
            "unit": "ns/iter"
          }
        ]
      },
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
          "id": "f0b50c022dfaf8dc6be25c4c9728e0403e445bfd",
          "message": "Merge pull request #103 from pgste/claude/reaper-compiled-input-design",
          "timestamp": "2026-07-20T22:39:07+01:00",
          "tree_id": "cf8f5544c8ecc9982a5d57fc8e0294e9bf914dcc",
          "url": "https://github.com/pgste/reaper/commit/f0b50c022dfaf8dc6be25c4c9728e0403e445bfd"
        },
        "date": 1784583880695,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_hot_swap/deploy_policy",
            "value": 183675,
            "range": "± 37666",
            "unit": "ns/iter"
          },
          {
            "name": "policy_hot_swap/concurrent_lookup",
            "value": 33,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "concurrent_access/concurrent_evaluations",
            "value": 345470,
            "range": "± 6245",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/100",
            "value": 179712,
            "range": "± 858",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/1000",
            "value": 3801705,
            "range": "± 5634",
            "unit": "ns/iter"
          },
          {
            "name": "realistic_workloads/microservice_auth_pattern",
            "value": 13187,
            "range": "± 32",
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
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/reachable_1hop_hit",
            "value": 57,
            "range": "± 0",
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
            "value": 23,
            "range": "± 1",
            "unit": "ns/iter"
          }
        ]
      },
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
          "id": "cb4b430c7180fcf3a61af0df1217dd8ec2a1fa7d",
          "message": "Merge pull request #102 from pgste/claude/reaper-r4-per-rule-fallback",
          "timestamp": "2026-07-21T06:31:48+01:00",
          "tree_id": "45792f1ef012a556d721f2f2894403125fbf1a58",
          "url": "https://github.com/pgste/reaper/commit/cb4b430c7180fcf3a61af0df1217dd8ec2a1fa7d"
        },
        "date": 1784612341998,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_hot_swap/deploy_policy",
            "value": 173041,
            "range": "± 41080",
            "unit": "ns/iter"
          },
          {
            "name": "policy_hot_swap/concurrent_lookup",
            "value": 47,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "concurrent_access/concurrent_evaluations",
            "value": 225084,
            "range": "± 2038",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/100",
            "value": 185283,
            "range": "± 229",
            "unit": "ns/iter"
          },
          {
            "name": "memory_efficiency/policy_storage/1000",
            "value": 3676579,
            "range": "± 7643",
            "unit": "ns/iter"
          },
          {
            "name": "realistic_workloads/microservice_auth_pattern",
            "value": 11876,
            "range": "± 91",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/has_relation_direct_hit",
            "value": 23,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/has_relation_direct_miss",
            "value": 23,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/reachable_1hop_hit",
            "value": 56,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/reachable_3hop_miss_bounded",
            "value": 56,
            "range": "± 1",
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