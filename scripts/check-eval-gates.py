#!/usr/bin/env python3
"""Regression gate checker for the Agentgrep public-v0.1 benchmark.

Reads summary.json produced by scripts/analyze-eval.py and enforces
minimum metric thresholds.  Exits non-zero if any gate fails.

Usage:
  # Either form works:
  python scripts/check-eval-gates.py --summary eval-results/<run-id>/summary.json
  python scripts/check-eval-gates.py --run-dir eval-results/<run-id>
"""
from __future__ import annotations

import argparse
import json
import os
import sys

# ---------------------------------------------------------------------------
# Gate definitions
# Each entry: (description, path-in-summary-json, op, threshold, explanation)
# op is ">=" or "==".
# ---------------------------------------------------------------------------

GATES = [
    # Mode B
    (
        "B hit@1",
        ("by_mode", "B", "hit@1"),
        ">=", 0.70,
        "Mode B (lexical, no index) must surface the correct file at rank 1 for >=70 % of tasks",
    ),
    (
        "B hit@8",
        ("by_mode", "B", "hit@8"),
        ">=", 0.85,
        "Mode B must surface the correct file within the top-8 results for >=85 % of tasks",
    ),
    # Mode C
    (
        "C hit@1",
        ("by_mode", "C", "hit@1"),
        ">=", 0.85,
        "Mode C (indexed fusion) must surface the correct file at rank 1 for >=85 % of tasks",
    ),
    (
        "C hit@8",
        ("by_mode", "C", "hit@8"),
        ">=", 1.00,
        "Mode C must find the correct file somewhere in the top-8 for every task",
    ),
    # Mode D
    (
        "D hit@1",
        ("by_mode", "D", "hit@1"),
        ">=", 0.90,
        "Mode D (semantic rerank) must surface the correct file at rank 1 for >=90 % of tasks",
    ),
    (
        "D hit@8",
        ("by_mode", "D", "hit@8"),
        ">=", 1.00,
        "Mode D must find the correct file somewhere in the top-8 for every task",
    ),
    (
        "D MRR",
        ("by_mode", "D", "mrr"),
        ">=", 0.95,
        "Mode D mean reciprocal rank must be >= 0.95",
    ),
    # Semantic safety (Mode D vs C comparison)
    (
        "semantic exact-query regression rate",
        ("semantic_c_vs_d", "overall", "exact_query_regression_rate"),
        "==", 0.0,
        "Semantic reranking must not push exact-query hits out of rank-1 (regression = 0)",
    ),
    (
        "semantic bad promotion rate",
        ("semantic_c_vs_d", "overall", "bad_promotion_rate"),
        "==", 0.0,
        "Semantic reranking must not promote irrelevant files into the top-8",
    ),
    # JSON parse success (all four modes)
    (
        "A JSON parse success rate",
        ("by_mode", "A", "json_parse_success_rate"),
        ">=", 1.0,
        "All Mode A responses must parse as valid JSON",
    ),
    (
        "B JSON parse success rate",
        ("by_mode", "B", "json_parse_success_rate"),
        ">=", 1.0,
        "All Mode B responses must parse as valid JSON",
    ),
    (
        "C JSON parse success rate",
        ("by_mode", "C", "json_parse_success_rate"),
        ">=", 1.0,
        "All Mode C responses must parse as valid JSON",
    ),
    (
        "D JSON parse success rate",
        ("by_mode", "D", "json_parse_success_rate"),
        ">=", 1.0,
        "All Mode D responses must parse as valid JSON",
    ),
]


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def get_nested(data: dict, path: tuple):
    cur = data
    for key in path:
        if not isinstance(cur, dict) or key not in cur:
            return None
        cur = cur[key]
    return cur


def check_gate(value, op: str, threshold: float) -> bool:
    if value is None:
        return False
    if op == ">=":
        return value >= threshold
    if op == "==":
        return value == threshold
    raise ValueError(f"unknown op: {op!r}")


def fmt_val(v) -> str:
    if v is None:
        return "MISSING"
    if isinstance(v, float):
        return f"{v:.4f}"
    return str(v)


def fmt_threshold(op: str, threshold: float) -> str:
    if threshold == int(threshold):
        return f"{op} {int(threshold)}"
    return f"{op} {threshold}"


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main(argv=None):
    ap = argparse.ArgumentParser(
        description="Check Agentgrep eval regression gates against summary.json.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=__doc__,
    )
    ap.add_argument(
        "--summary",
        metavar="PATH",
        help="Path to summary.json (e.g. eval-results/<run-id>/summary.json).",
    )
    ap.add_argument(
        "--run-dir",
        metavar="DIR",
        help="Run directory; summary.json is inferred as <run-dir>/summary.json.",
    )
    args = ap.parse_args(argv)

    summary_path = args.summary
    if not summary_path:
        if not args.run_dir:
            ap.error("provide --summary or --run-dir")
        summary_path = os.path.join(args.run_dir, "summary.json")

    if not os.path.exists(summary_path):
        sys.exit(f"ERROR: summary.json not found: {summary_path}")

    with open(summary_path, "r", encoding="utf-8") as fh:
        try:
            data = json.load(fh)
        except json.JSONDecodeError as exc:
            sys.exit(f"ERROR: cannot parse {summary_path}: {exc}")

    # Evaluate gates.
    rows = []
    for name, path, op, threshold, explanation in GATES:
        value = get_nested(data, path)
        passed = check_gate(value, op, threshold)
        rows.append((name, value, op, threshold, explanation, passed))

    # Print table.
    col_name = max(len(r[0]) for r in rows)
    col_actual = max(len(fmt_val(r[1])) for r in rows)

    header = (
        f"{'Gate':<{col_name}}  {'Actual':>{col_actual}}  {'Threshold':<12}  Status"
    )
    sep = "-" * len(header)
    print(f"\n{sep}")
    print(header)
    print(sep)

    failures = []
    for name, value, op, threshold, explanation, passed in rows:
        status = "PASS" if passed else "FAIL"
        print(
            f"{name:<{col_name}}  {fmt_val(value):>{col_actual}}  "
            f"{fmt_threshold(op, threshold):<12}  {status}"
        )
        if not passed:
            failures.append((name, value, fmt_threshold(op, threshold), explanation))

    print(sep)

    if failures:
        print(f"\nFAILED: {len(failures)} gate(s) did not meet threshold:\n")
        for name, value, threshold_str, explanation in failures:
            print(f"  [{name}]  actual={fmt_val(value)}  required={threshold_str}")
            print(f"    {explanation}\n")
        print(f"eval-gates: FAIL ({len(failures)} failure(s))")
        sys.exit(1)
    else:
        print(f"\neval-gates: PASS (all {len(rows)} gates met)")
        sys.exit(0)


if __name__ == "__main__":
    main()
