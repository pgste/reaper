window.BENCHMARK_DATA = {
  "lastUpdate": 1784759723779,
  "repoUrl": "https://github.com/pgste/reaper",
  "entries": {
    "Eval latency (criterion)": [
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
        "date": 1784541121848,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 93,
            "range": "± 7",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 227,
            "range": "± 10",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 93,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 210,
            "range": "± 11",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 358,
            "range": "± 5",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 813,
            "range": "± 42",
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
        "date": 1784541158461,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 121,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 446,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 122,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 302,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 562,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1292,
            "range": "± 9",
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
        "date": 1784551689201,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 125,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 378,
            "range": "± 4",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 123,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 282,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 509,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1138,
            "range": "± 11",
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
        "date": 1784583572870,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 126,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 407,
            "range": "± 3",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 124,
            "range": "± 8",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 285,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 518,
            "range": "± 5",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1153,
            "range": "± 9",
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
        "date": 1784583585543,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 121,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 488,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 121,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 302,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 555,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1315,
            "range": "± 12",
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
        "date": 1784583660716,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 107,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 351,
            "range": "± 3",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 106,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 244,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 440,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 998,
            "range": "± 15",
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
        "date": 1784583878031,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 134,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 476,
            "range": "± 11",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 134,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 331,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 594,
            "range": "± 23",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1339,
            "range": "± 6",
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
        "date": 1784612339013,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 117,
            "range": "± 3",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 354,
            "range": "± 3",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 115,
            "range": "± 3",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 291,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 541,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1173,
            "range": "± 4",
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
          "id": "74c5abc873b91a29ab5096863487244ad62fc17f",
          "message": "Merge pull request #105 from pgste/claude/reaper-r4-a3-to-main",
          "timestamp": "2026-07-21T13:10:14+01:00",
          "tree_id": "6ae6b6e33d85b26d59d04b4afb9d686fe695d683",
          "url": "https://github.com/pgste/reaper/commit/74c5abc873b91a29ab5096863487244ad62fc17f"
        },
        "date": 1784636095493,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 121,
            "range": "± 5",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 466,
            "range": "± 6",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 122,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 301,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 553,
            "range": "± 8",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1274,
            "range": "± 4",
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
          "id": "1a9e23804cdc04dee894a8f0c7f8ce4e8a603a5f",
          "message": "Merge pull request #107 from pgste/claude/reaper-r4-b2-input-comprehensions",
          "timestamp": "2026-07-21T17:50:49+01:00",
          "tree_id": "20d36e7ea80ab0f529e71ccf6905ec680ebf2ce7",
          "url": "https://github.com/pgste/reaper/commit/1a9e23804cdc04dee894a8f0c7f8ce4e8a603a5f"
        },
        "date": 1784652944412,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 133,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 484,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 134,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 338,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 611,
            "range": "± 8",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1345,
            "range": "± 30",
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
          "id": "8b3a2b110e4dee421d56a74c6d083cab6f5d46d6",
          "message": "Merge pull request #108 from pgste/claude/reaper-r4-b2b-stringop-filters",
          "timestamp": "2026-07-21T21:36:15+01:00",
          "tree_id": "839e99b85c3dd334a67b505dfbd777111a2d61eb",
          "url": "https://github.com/pgste/reaper/commit/8b3a2b110e4dee421d56a74c6d083cab6f5d46d6"
        },
        "date": 1784666477800,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 130,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 431,
            "range": "± 8",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 128,
            "range": "± 3",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 292,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 532,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1171,
            "range": "± 6",
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
          "id": "64e12ddc81a3bfdf4400f22fccdf55cb35209ba7",
          "message": "Merge pull request #109 from pgste/claude/reaper-r4-b3-check-driver",
          "timestamp": "2026-07-22T23:30:02+01:00",
          "tree_id": "65c5cb2a6ddd71735fa85f64e8a06233e3d15450",
          "url": "https://github.com/pgste/reaper/commit/64e12ddc81a3bfdf4400f22fccdf55cb35209ba7"
        },
        "date": 1784759723161,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 134,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 474,
            "range": "± 8",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 134,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 341,
            "range": "± 4",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 605,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1351,
            "range": "± 7",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}