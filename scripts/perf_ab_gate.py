#!/usr/bin/env python3
"""Paired A/B criterion benchmark gate (Plan 08 Phase F).

Compares two criterion baselines ("base" = merge-base, "head" = PR head)
benchmarked IN THE SAME CI JOB ON THE SAME RUNNER, so machine-to-machine
variance — the reason a naive shared-runner gate needed a 30% threshold —
cancels out by construction. What remains is within-run jitter, which
criterion's confidence intervals quantify.

A benchmark REGRESSES when BOTH hold:
  1. head mean > base mean * threshold (10% for the stable eval-latency set,
     50% for variance-prone micro-ops like hot-swap/threaded benches), AND
  2. the 95% confidence intervals do not overlap (head lower bound > base
     upper bound) — i.e. the regression is outside measurement noise.

Exit code 1 if any benchmark regresses; the offending rows are listed.

`--self-test` builds synthetic baseline trees and asserts the gate's verdicts
(including that a clean +15% regression on the tight set FAILS) — CI runs it
before the real comparison, so the gate's sensitivity is proven on every run.

HTTP A/B mode (`--http-ab`): compares externally-produced latency samples
instead of criterion baselines — used by the served-path SLO A/B gate
(slo-harness runs interleaved base/head against real agents on the same
runner). Each of --base-json/--head-json is a JSON array, either of numbers
or of objects carrying --metric (default `p99_us`, the slo-harness result
field). A regression requires BOTH:
  1. median(head) > median(base) * threshold (default 1.25 — HTTP loopback
     jitters more than criterion's in-process benches), AND
  2. min(head) > max(base) — every head run is slower than every base run,
     the sample-based analog of non-overlapping confidence intervals.
"""

import argparse
import json
import os
import re
import sys
import tempfile

# Stable, allocation-light end-to-end evaluation benches: tight gate.
# Everything else (hot-swap deploys, threaded contention, memory/realistic
# macro benches) jitters run-to-run even on one machine: loose gate.
DEFAULT_TIGHT_PATTERN = r"^(policy_evaluation|latency_targets|rebac)/"
DEFAULT_TIGHT_THRESHOLD = 1.10
DEFAULT_LOOSE_THRESHOLD = 1.50


def load_estimate(path):
    """Return (mean, ci_lower, ci_upper) from a criterion estimates.json."""
    with open(path) as f:
        mean = json.load(f)["mean"]
    ci = mean["confidence_interval"]
    return mean["point_estimate"], ci["lower_bound"], ci["upper_bound"]


def collect_baseline(criterion_dir, baseline):
    """Map benchmark id -> estimates path for one saved baseline.

    Criterion stores a saved baseline at
    `<criterion_dir>/<bench...>/<baseline>/estimates.json`; the benchmark id
    is the path between the criterion dir and the baseline dir.
    """
    found = {}
    for root, _dirs, files in os.walk(criterion_dir):
        if os.path.basename(root) == baseline and "estimates.json" in files:
            bench_id = os.path.relpath(os.path.dirname(root), criterion_dir)
            found[bench_id.replace(os.sep, "/")] = os.path.join(root, "estimates.json")
    return found


def compare(criterion_dir, base, head, tight_re, tight_threshold, loose_threshold):
    """Return (rows, regressions). Each row is a dict for reporting."""
    base_estimates = collect_baseline(criterion_dir, base)
    head_estimates = collect_baseline(criterion_dir, head)
    shared = sorted(set(base_estimates) & set(head_estimates))

    rows, regressions = [], []
    for bench_id in shared:
        b_mean, _b_lo, b_hi = load_estimate(base_estimates[bench_id])
        h_mean, h_lo, _h_hi = load_estimate(head_estimates[bench_id])
        if b_mean <= 0:
            continue
        ratio = h_mean / b_mean
        tight = bool(tight_re.search(bench_id))
        threshold = tight_threshold if tight else loose_threshold
        outside_noise = h_lo > b_hi
        regressed = ratio > threshold and outside_noise
        row = {
            "bench": bench_id,
            "base_ns": b_mean,
            "head_ns": h_mean,
            "ratio": ratio,
            "gate": f"{threshold:.2f}x ({'tight' if tight else 'loose'})",
            "regressed": regressed,
        }
        rows.append(row)
        if regressed:
            regressions.append(row)
    return rows, regressions


def report(rows, regressions, base_count, head_count):
    if not rows:
        print("ERROR: no benchmarks present in BOTH baselines "
              f"(base has {base_count}, head has {head_count}) — "
              "the comparison ran on nothing, refusing to pass vacuously.")
        return 1

    rows.sort(key=lambda r: r["ratio"], reverse=True)
    print(f"{'benchmark':60} {'base':>12} {'head':>12} {'ratio':>7}  gate")
    for r in rows:
        flag = "  << REGRESSED" if r["regressed"] else ""
        print(f"{r['bench']:60} {r['base_ns']:12.1f} {r['head_ns']:12.1f} "
              f"{r['ratio']:7.3f}  {r['gate']}{flag}")

    if regressions:
        print(f"\nFAIL: {len(regressions)} benchmark(s) regressed beyond their "
              "gate with non-overlapping confidence intervals:")
        for r in regressions:
            print(f"  - {r['bench']}: {r['ratio']:.3f}x (gate {r['gate']})")
        return 1
    print(f"\nOK: {len(rows)} benchmark(s) within gates.")
    return 0


# ---------------------------------------------------------------------------
# HTTP A/B mode — externally-produced latency samples (slo-harness runs).
# ---------------------------------------------------------------------------

def _load_samples(path, metric):
    """Load a JSON array of numbers, or of objects carrying `metric`."""
    with open(path) as f:
        data = json.load(f)
    if not isinstance(data, list) or not data:
        raise ValueError(f"{path}: expected a non-empty JSON array")
    samples = []
    for item in data:
        if isinstance(item, (int, float)):
            samples.append(float(item))
        elif isinstance(item, dict) and metric in item:
            samples.append(float(item[metric]))
        else:
            raise ValueError(f"{path}: entry {item!r} has no metric '{metric}'")
    return samples


def _median(xs):
    s = sorted(xs)
    n = len(s)
    return s[n // 2] if n % 2 else (s[n // 2 - 1] + s[n // 2]) / 2


def compare_http_samples(base, head, threshold):
    """Return (report_lines, regressed) for two lists of latency samples.

    Regression requires BOTH median(head) > median(base) * threshold AND
    min(head) > max(base) (all head runs slower than all base runs) — a
    sample-based analog of the criterion CI-overlap rule, so run-to-run HTTP
    jitter cannot fail the gate on its own.
    """
    b_med, h_med = _median(base), _median(head)
    ratio = h_med / b_med if b_med > 0 else float("inf")
    outside_noise = min(head) > max(base)
    regressed = ratio > threshold and outside_noise
    lines = [
        f"base runs: {sorted(base)} (median {b_med:.1f})",
        f"head runs: {sorted(head)} (median {h_med:.1f})",
        f"ratio: {ratio:.3f} (gate {threshold:.2f}x), "
        f"outside noise: {outside_noise} (min(head) {min(head):.1f} "
        f"{'>' if outside_noise else '<='} max(base) {max(base):.1f})",
    ]
    return lines, regressed


def http_ab(base_path, head_path, metric, threshold):
    base = _load_samples(base_path, metric)
    head = _load_samples(head_path, metric)
    lines, regressed = compare_http_samples(base, head, threshold)
    print(f"HTTP A/B gate on '{metric}':")
    for line in lines:
        print(f"  {line}")
    if regressed:
        print(f"\nFAIL: served-path {metric} regressed beyond {threshold:.2f}x "
              "with all head runs slower than all base runs.")
        return 1
    print(f"\nOK: served-path {metric} within gate.")
    return 0


# ---------------------------------------------------------------------------
# Self-test: synthetic baselines proving the gate's verdicts, run in CI
# before every real comparison.
# ---------------------------------------------------------------------------

def _write_estimate(criterion_dir, bench_id, baseline, mean, lo, hi):
    d = os.path.join(criterion_dir, bench_id, baseline)
    os.makedirs(d, exist_ok=True)
    with open(os.path.join(d, "estimates.json"), "w") as f:
        json.dump({"mean": {
            "point_estimate": mean,
            "confidence_interval": {"lower_bound": lo, "upper_bound": hi},
        }}, f)


def self_test():
    tight_re = re.compile(DEFAULT_TIGHT_PATTERN)
    cases_failing = {
        # A clean +15% on the tight set MUST fail (the plan's DoD case).
        "policy_evaluation/regressed_15pct": (100, 99, 101, 115, 114, 116),
        # A gross +60% on the loose set must fail too.
        "policy_hot_swap/regressed_60pct": (100, 99, 101, 160, 158, 162),
    }
    cases_passing = {
        # +15% point estimate but the intervals overlap: noise, not regression.
        "policy_evaluation/noisy_15pct": (100, 80, 120, 115, 95, 135),
        # +5% is under the tight gate.
        "policy_evaluation/small_5pct": (100, 99, 101, 105, 104, 106),
        # +30% on the loose set is under its 50% gate.
        "policy_hot_swap/loose_30pct": (100, 99, 101, 130, 129, 131),
        # An improvement, obviously fine.
        "rebac/improved": (100, 99, 101, 80, 79, 81),
    }

    with tempfile.TemporaryDirectory() as tmp:
        for bench_id, (bm, bl, bh, hm, hl, hh) in {**cases_failing, **cases_passing}.items():
            _write_estimate(tmp, bench_id, "base", bm, bl, bh)
            _write_estimate(tmp, bench_id, "head", hm, hl, hh)

        rows, regressions = compare(
            tmp, "base", "head", tight_re,
            DEFAULT_TIGHT_THRESHOLD, DEFAULT_LOOSE_THRESHOLD,
        )
        assert len(rows) == len(cases_failing) + len(cases_passing), \
            f"expected {len(cases_failing) + len(cases_passing)} compared rows, got {len(rows)}"
        got_failing = {r["bench"] for r in regressions}
        assert got_failing == set(cases_failing), (
            f"gate verdicts wrong: expected {sorted(cases_failing)} to regress, "
            f"got {sorted(got_failing)}"
        )

        # An empty comparison must be an error, never a vacuous pass.
        assert report([], [], 0, 0) == 1

    # HTTP A/B mode verdicts (served-path SLO gate).
    # Clean +50% with disjoint samples: regression.
    _, regressed = compare_http_samples([100, 102, 98], [150, 155, 148], 1.25)
    assert regressed, "clean +50% disjoint HTTP regression must fail"
    # +50% median but overlapping samples (one head run as fast as base): noise.
    _, regressed = compare_http_samples([100, 102, 98], [99, 150, 155], 1.25)
    assert not regressed, "overlapping HTTP samples must not fail the gate"
    # +10% is under the 1.25x gate even when disjoint.
    _, regressed = compare_http_samples([100, 102, 98], [110, 112, 108], 1.25)
    assert not regressed, "+10% under the HTTP gate threshold must pass"
    # An improvement passes.
    _, regressed = compare_http_samples([100, 102, 98], [80, 82, 78], 1.25)
    assert not regressed, "an HTTP improvement must pass"

    print("self-test ok: +15% tight regression fails, noise/small/loose-30% pass, "
          "empty comparison is an error; http-ab: disjoint +50% fails, "
          "overlap/under-threshold/improvement pass")
    return 0


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--criterion-dir", default="target/criterion")
    p.add_argument("--base", default="base")
    p.add_argument("--head", default="head")
    p.add_argument("--tight-pattern", default=DEFAULT_TIGHT_PATTERN)
    p.add_argument("--tight-threshold", type=float, default=DEFAULT_TIGHT_THRESHOLD)
    p.add_argument("--loose-threshold", type=float, default=DEFAULT_LOOSE_THRESHOLD)
    p.add_argument("--self-test", action="store_true")
    p.add_argument("--http-ab", action="store_true",
                   help="compare externally-produced latency samples "
                        "(slo-harness runs) instead of criterion baselines")
    p.add_argument("--base-json", help="http-ab: JSON array of base-side runs")
    p.add_argument("--head-json", help="http-ab: JSON array of head-side runs")
    p.add_argument("--metric", default="p99_us",
                   help="http-ab: object key holding the latency value")
    p.add_argument("--http-threshold", type=float, default=1.25,
                   help="http-ab: median-ratio gate (looser than criterion's "
                        "10%% because HTTP loopback jitters more)")
    args = p.parse_args()

    if args.self_test:
        return self_test()

    if args.http_ab:
        if not args.base_json or not args.head_json:
            p.error("--http-ab requires --base-json and --head-json")
        return http_ab(args.base_json, args.head_json, args.metric,
                       args.http_threshold)

    tight_re = re.compile(args.tight_pattern)
    base_count = len(collect_baseline(args.criterion_dir, args.base))
    head_count = len(collect_baseline(args.criterion_dir, args.head))
    rows, regressions = compare(
        args.criterion_dir, args.base, args.head, tight_re,
        args.tight_threshold, args.loose_threshold,
    )
    return report(rows, regressions, base_count, head_count)


if __name__ == "__main__":
    sys.exit(main())
