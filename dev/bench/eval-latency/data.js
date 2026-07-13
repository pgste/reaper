window.BENCHMARK_DATA = {
  "lastUpdate": 1783952789338,
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
          "id": "f8852af7219e44c876bb7cd2426eb314b1b5dfb4",
          "message": "Merge pull request #50 from pgste/claude/reaper-security-perf-review-9pz54s",
          "timestamp": "2026-07-13T14:13:44+01:00",
          "tree_id": "4bd39c59ee829c0137407e0c1ae056bb07077afc",
          "url": "https://github.com/pgste/reaper/commit/f8852af7219e44c876bb7cd2426eb314b1b5dfb4"
        },
        "date": 1783948712004,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 119,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 479,
            "range": "± 7",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 118,
            "range": "± 6",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 300,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 554,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1313,
            "range": "± 18",
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
          "id": "27c5b220a978698eff4315f376a885b1c72edfee",
          "message": "Merge pull request #51 from pgste/claude/reaper-security-perf-review-9pz54s",
          "timestamp": "2026-07-13T15:21:49+01:00",
          "tree_id": "86bbb09bc4a0bd048cf76f452418ddc253193e46",
          "url": "https://github.com/pgste/reaper/commit/27c5b220a978698eff4315f376a885b1c72edfee"
        },
        "date": 1783952788896,
        "tool": "cargo",
        "benches": [
          {
            "name": "policy_evaluation/simple_policy",
            "value": 118,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "policy_evaluation/complex_policy",
            "value": 478,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "latency_targets/policy_evaluation_performance",
            "value": 119,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_allow_group_viewer",
            "value": 303,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/compiled_policy_deny_full_sweep",
            "value": 554,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "rebac/ast_policy_allow_group_viewer",
            "value": 1336,
            "range": "± 11",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}