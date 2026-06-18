#!/usr/bin/env python3
"""Compute Agentgrep public-benchmark retrieval metrics from a run.

Reads the structured mode output produced by scripts/run-eval.ps1
(eval-results/<run-id>/parsed/results.jsonl) together with the relevance
labels, and writes summary.csv and summary.json grouped by repo, task type,
and mode.

Metrics (see docs/evaluation/METRICS.md):
  Retrieval (per mode): Hit@1, Hit@3, Hit@8, MRR, nDCG@8, Precision@8,
  Recall@8, mean unnecessary files before first hit, JSON parse success,
  latency p50/p95.
  Semantic (Mode D vs Mode C): semantic-only helpful hit rate, bad promotion
  rate, exact-query regression rate, latency delta p50/p95.

Usage:
  python scripts/analyze-eval.py --run-dir eval-results/<run-id> \
      --labels docs/evaluation/labels/public-v0.1.jsonl

  # Validate task/label data without a run:
  python scripts/analyze-eval.py --validate \
      --tasks docs/evaluation/tasks/public-v0.1.jsonl \
      --labels docs/evaluation/labels/public-v0.1.jsonl
"""
from __future__ import annotations

import argparse
import csv
import json
import math
import os
import sys
from collections import defaultdict

# Label -> graded gain used for nDCG.
GAIN = {"primary": 3.0, "acceptable": 2.0, "supporting": 1.0, "irrelevant": 0.0}
HIT_LABELS = {"primary", "acceptable"}
RELEVANT_LABELS = {"primary", "acceptable", "supporting"}
VALID_LABELS = set(GAIN.keys())
K = 8  # top-k cutoff for @8 metrics and semantic comparison


# --------------------------------------------------------------------------
# IO helpers
# --------------------------------------------------------------------------

def read_jsonl(path):
    rows = []
    with open(path, "r", encoding="utf-8-sig") as fh:
        for n, line in enumerate(fh, 1):
            line = line.strip()
            if not line:
                continue
            try:
                rows.append(json.loads(line))
            except json.JSONDecodeError as exc:
                raise SystemExit(f"{path}:{n}: invalid JSON: {exc}")
    return rows


def norm_path(p):
    if p is None:
        return None
    s = str(p).replace("\\", "/")
    if s.startswith("./"):
        s = s[2:]
    return s


def coerce_paths(value):
    if value is None:
        return []
    if isinstance(value, str):
        return [norm_path(value)]
    if isinstance(value, list):
        return [norm_path(v) for v in value if v is not None]
    return []


def load_labels(path):
    """task_id -> {norm_path: label}."""
    out = {}
    for row in read_jsonl(path):
        tid = row.get("task_id")
        labels = {}
        for entry in row.get("labels", []) or []:
            p = norm_path(entry.get("path"))
            lab = entry.get("label")
            if p is not None and lab:
                labels[p] = lab
        out[tid] = labels
    return out


# --------------------------------------------------------------------------
# Metric math
# --------------------------------------------------------------------------

def percentile(values, pct):
    vals = sorted(v for v in values if v is not None)
    if not vals:
        return None
    if len(vals) == 1:
        return float(vals[0])
    rank = (pct / 100.0) * (len(vals) - 1)
    lo = math.floor(rank)
    hi = math.ceil(rank)
    if lo == hi:
        return float(vals[lo])
    frac = rank - lo
    return float(vals[lo] + (vals[hi] - vals[lo]) * frac)


def first_hit_rank(ranked, labels):
    """1-based rank of the first primary/acceptable file, or None."""
    for i, p in enumerate(ranked, 1):
        if labels.get(p) in HIT_LABELS:
            return i
    return None


def ndcg_at_k(ranked, labels, k=K):
    gains = [GAIN.get(labels.get(p), 0.0) for p in ranked[:k]]
    dcg = sum(g / math.log2(i + 1) for i, g in enumerate(gains, 1))
    ideal_gains = sorted(
        (GAIN.get(l, 0.0) for l in labels.values()), reverse=True
    )[:k]
    idcg = sum(g / math.log2(i + 1) for i, g in enumerate(ideal_gains, 1))
    if idcg == 0:
        return None  # no graded-relevant docs; nDCG undefined
    return dcg / idcg


def per_task_metrics(ranked, labels):
    """Compute retrieval metrics for one (task, mode) ranked list."""
    relevant = {p for p, l in labels.items() if l in RELEVANT_LABELS}
    topk = ranked[:K]
    fhr = first_hit_rank(ranked, labels)
    n_rel_in_topk = sum(1 for p in topk if p in relevant)
    precision = (n_rel_in_topk / len(topk)) if topk else 0.0
    recall = (n_rel_in_topk / len(relevant)) if relevant else None
    return {
        "hit@1": 1.0 if (fhr is not None and fhr <= 1) else 0.0,
        "hit@3": 1.0 if (fhr is not None and fhr <= 3) else 0.0,
        "hit@8": 1.0 if (fhr is not None and fhr <= 8) else 0.0,
        "rr": (1.0 / fhr) if fhr else 0.0,
        "ndcg@8": ndcg_at_k(ranked, labels),
        "precision@8": precision,
        "recall@8": recall,
        "unnecessary_before_first_hit": (fhr - 1) if fhr else None,
        "miss": fhr is None,
    }


def mean(values):
    vals = [v for v in values if v is not None]
    return (sum(vals) / len(vals)) if vals else None


# --------------------------------------------------------------------------
# Aggregation
# --------------------------------------------------------------------------

def aggregate(per_task_rows):
    """per_task_rows: list of dicts with metric fields + latency/json flags.
    Returns an aggregate dict for the group."""
    n = len(per_task_rows)
    misses = sum(1 for r in per_task_rows if r["miss"])
    return {
        "n": n,
        "hit@1": mean(r["hit@1"] for r in per_task_rows),
        "hit@3": mean(r["hit@3"] for r in per_task_rows),
        "hit@8": mean(r["hit@8"] for r in per_task_rows),
        "mrr": mean(r["rr"] for r in per_task_rows),
        "ndcg@8": mean(r["ndcg@8"] for r in per_task_rows),
        "precision@8": mean(r["precision@8"] for r in per_task_rows),
        "recall@8": mean(r["recall@8"] for r in per_task_rows),
        "mean_unnecessary_before_first_hit": mean(
            r["unnecessary_before_first_hit"] for r in per_task_rows
        ),
        "misses": misses,
        "json_parse_success_rate": mean(
            (1.0 if r["json_parse_ok"] else 0.0) for r in per_task_rows
        ),
        "latency_p50_ms": percentile([r["latency_ms"] for r in per_task_rows], 50),
        "latency_p95_ms": percentile([r["latency_ms"] for r in per_task_rows], 95),
    }


def round_metrics(d):
    out = {}
    for k, v in d.items():
        if isinstance(v, float):
            out[k] = round(v, 4)
        else:
            out[k] = v
    return out


# --------------------------------------------------------------------------
# Main analysis
# --------------------------------------------------------------------------

def analyze(results, labels_by_task):
    # Scored per-task rows keyed for grouping; also keep C/D ranked lists for
    # the semantic comparison.
    scored = []
    cd_index = defaultdict(dict)  # task_id -> {mode: record+metrics}

    skipped_no_labels = []
    for rec in results:
        if rec.get("skipped"):
            continue
        tid = rec.get("task_id")
        labels = labels_by_task.get(tid)
        if not labels:
            skipped_no_labels.append((tid, rec.get("mode")))
            continue
        ranked = coerce_paths(rec.get("ranked_paths"))
        m = per_task_metrics(ranked, labels)
        row = dict(m)
        row.update(
            {
                "task_id": tid,
                "repo_id": rec.get("repo_id"),
                "task_type": rec.get("task_type"),
                "mode": rec.get("mode"),
                "latency_ms": rec.get("latency_ms"),
                "json_parse_ok": bool(rec.get("json_parse_ok")),
                "ranked": ranked,
            }
        )
        scored.append(row)
        cd_index[tid][rec.get("mode")] = row

    # Group aggregations.
    groups = {"by_mode": defaultdict(list), "by_repo_mode": defaultdict(list),
              "by_tasktype_mode": defaultdict(list)}
    for r in scored:
        groups["by_mode"][r["mode"]].append(r)
        groups["by_repo_mode"][(r["repo_id"], r["mode"])].append(r)
        groups["by_tasktype_mode"][(r["task_type"], r["mode"])].append(r)

    summary = {
        "by_mode": {},
        "by_repo_mode": {},
        "by_tasktype_mode": {},
        "semantic_c_vs_d": {},
    }
    for mode, rows in sorted(groups["by_mode"].items()):
        summary["by_mode"][mode] = round_metrics(aggregate(rows))
    for (repo, mode), rows in sorted(groups["by_repo_mode"].items()):
        summary["by_repo_mode"].setdefault(repo, {})[mode] = round_metrics(aggregate(rows))
    for (tt, mode), rows in sorted(groups["by_tasktype_mode"].items()):
        summary["by_tasktype_mode"].setdefault(tt, {})[mode] = round_metrics(aggregate(rows))

    # Semantic comparison (Mode D vs Mode C).
    summary["semantic_c_vs_d"] = semantic_comparison(cd_index, labels_by_task)

    return summary, scored, skipped_no_labels


def semantic_comparison(cd_index, labels_by_task):
    overall = []
    by_type = defaultdict(list)
    for tid, modes in cd_index.items():
        if "C" not in modes or "D" not in modes:
            continue
        c, d = modes["C"], modes["D"]
        labels = labels_by_task.get(tid, {})
        c_top = set(c["ranked"][:K])
        d_top = set(d["ranked"][:K])
        hit_paths = {p for p, l in labels.items() if l in HIT_LABELS}
        irrelevant_paths = {p for p, l in labels.items() if l == "irrelevant"}

        c_has_hit = bool(hit_paths & c_top)
        d_has_hit = bool(hit_paths & d_top)
        helpful = d_has_hit and not c_has_hit
        bad_promo = bool((irrelevant_paths & d_top) - (irrelevant_paths & c_top))
        regression = (c["hit@1"] == 1.0) and (d["hit@1"] != 1.0)
        lat_delta = None
        if c["latency_ms"] is not None and d["latency_ms"] is not None:
            lat_delta = d["latency_ms"] - c["latency_ms"]

        row = {
            "task_id": tid,
            "task_type": d["task_type"],
            "semantic_only_helpful_hit": helpful,
            "bad_promotion": bad_promo,
            "exact_query_regression": regression,
            "latency_delta_ms": lat_delta,
        }
        overall.append(row)
        by_type[d["task_type"]].append(row)

    def summarize(rows):
        if not rows:
            return {"n": 0}
        return round_metrics({
            "n": len(rows),
            "semantic_only_helpful_hit_rate": mean(
                1.0 if r["semantic_only_helpful_hit"] else 0.0 for r in rows
            ),
            "bad_promotion_rate": mean(
                1.0 if r["bad_promotion"] else 0.0 for r in rows
            ),
            "exact_query_regression_rate": mean(
                1.0 if r["exact_query_regression"] else 0.0 for r in rows
            ),
            "latency_delta_p50_ms": percentile(
                [r["latency_delta_ms"] for r in rows], 50
            ),
            "latency_delta_p95_ms": percentile(
                [r["latency_delta_ms"] for r in rows], 95
            ),
        })

    return {
        "overall": summarize(overall),
        "by_task_type": {tt: summarize(rows) for tt, rows in sorted(by_type.items())},
    }


# --------------------------------------------------------------------------
# Output writers
# --------------------------------------------------------------------------

METRIC_COLS = [
    "n", "hit@1", "hit@3", "hit@8", "mrr", "ndcg@8", "precision@8", "recall@8",
    "mean_unnecessary_before_first_hit", "misses", "json_parse_success_rate",
    "latency_p50_ms", "latency_p95_ms",
]


def write_csv(summary, path):
    with open(path, "w", encoding="utf-8", newline="") as fh:
        w = csv.writer(fh)
        w.writerow(["group_kind", "group_key", "mode"] + METRIC_COLS)
        for mode, m in summary["by_mode"].items():
            w.writerow(["overall", "all", mode] + [m.get(c) for c in METRIC_COLS])
        for repo, modes in summary["by_repo_mode"].items():
            for mode, m in modes.items():
                w.writerow(["repo", repo, mode] + [m.get(c) for c in METRIC_COLS])
        for tt, modes in summary["by_tasktype_mode"].items():
            for mode, m in modes.items():
                w.writerow(["task_type", tt, mode] + [m.get(c) for c in METRIC_COLS])

        # Semantic comparison rows.
        sem = summary["semantic_c_vs_d"]
        sem_cols = ["n", "semantic_only_helpful_hit_rate", "bad_promotion_rate",
                    "exact_query_regression_rate", "latency_delta_p50_ms",
                    "latency_delta_p95_ms"]
        w.writerow([])
        w.writerow(["semantic_group", "key"] + sem_cols)
        w.writerow(["semantic_overall", "all"] + [sem["overall"].get(c) for c in sem_cols])
        for tt, m in sem["by_task_type"].items():
            w.writerow(["semantic_task_type", tt] + [m.get(c) for c in sem_cols])


def print_console(summary):
    print("\n=== Retrieval metrics by mode (see summary.csv/json for full breakdown) ===")
    header = f"{'mode':<5}{'n':>4}{'hit@1':>8}{'hit@3':>8}{'hit@8':>8}{'mrr':>8}{'ndcg@8':>9}{'p@8':>7}{'r@8':>7}{'p50ms':>10}"
    print(header)
    for mode, m in summary["by_mode"].items():
        def f(x):
            return f"{x:.3f}" if isinstance(x, float) else ("-" if x is None else str(x))
        def fi(x):
            return "-" if x is None else f"{x:.0f}"
        print(f"{mode:<5}{m['n']:>4}{f(m['hit@1']):>8}{f(m['hit@3']):>8}"
              f"{f(m['hit@8']):>8}{f(m['mrr']):>8}{f(m['ndcg@8']):>9}"
              f"{f(m['precision@8']):>7}{f(m['recall@8']):>7}{fi(m['latency_p50_ms']):>10}")
    sem = summary["semantic_c_vs_d"]["overall"]
    if sem.get("n"):
        print("\n=== Semantic (Mode D vs C) overall ===")
        print(f"  paired tasks:            {sem['n']}")
        print(f"  semantic-only helpful:   {sem.get('semantic_only_helpful_hit_rate')}")
        print(f"  bad promotion rate:      {sem.get('bad_promotion_rate')}")
        print(f"  exact-query regression:  {sem.get('exact_query_regression_rate')}")
        print(f"  latency delta p50/p95:   {sem.get('latency_delta_p50_ms')} / {sem.get('latency_delta_p95_ms')} ms")


# --------------------------------------------------------------------------
# Validation mode
# --------------------------------------------------------------------------

def validate(tasks_path, labels_path):
    tasks = read_jsonl(tasks_path)
    task_by_id = {t.get("task_id"): t for t in tasks}
    label_rows = read_jsonl(labels_path)
    errors, warnings = [], []

    for t in tasks:
        if not t.get("task_id"):
            errors.append("task missing task_id")
        if not t.get("repo_id"):
            errors.append(f"{t.get('task_id')}: missing repo_id")
        if not t.get("query"):
            errors.append(f"{t.get('task_id')}: missing query")

    labeled_ids = set()
    for row in label_rows:
        tid = row.get("task_id")
        labeled_ids.add(tid)
        if tid not in task_by_id:
            errors.append(f"label references unknown task_id: {tid}")
        else:
            if row.get("repo_id") != task_by_id[tid].get("repo_id"):
                errors.append(f"{tid}: label repo_id != task repo_id")
        for entry in row.get("labels", []) or []:
            p = entry.get("path", "")
            lab = entry.get("label")
            if lab not in VALID_LABELS:
                errors.append(f"{tid}: invalid label '{lab}' for {p}")
            if not p:
                errors.append(f"{tid}: label entry missing path")
            elif p.startswith("/") or "\\" in p or (len(p) > 1 and p[1] == ":"):
                errors.append(f"{tid}: path must be repo-relative forward-slash: {p}")

    for tid in task_by_id:
        if tid not in labeled_ids and not tid.upper().startswith("EXAMPLE"):
            warnings.append(f"{tid}: no labels (task will be skipped by analyzer)")

    print(f"tasks: {len(tasks)}  labeled tasks: {len(labeled_ids)}")
    for w in warnings:
        print(f"  WARN  {w}")
    for e in errors:
        print(f"  ERROR {e}")
    if errors:
        print(f"\nVALIDATION FAILED: {len(errors)} error(s)")
        return 1
    print("\nVALIDATION OK")
    return 0


# --------------------------------------------------------------------------
# CLI
# --------------------------------------------------------------------------

def main(argv=None):
    ap = argparse.ArgumentParser(
        description="Compute Agentgrep public-benchmark retrieval metrics.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    ap.add_argument("--run-dir", help="Run directory (contains parsed/results.jsonl).")
    ap.add_argument("--results", help="Explicit path to results.jsonl (overrides --run-dir).")
    ap.add_argument("--labels", help="Path to label JSONL.")
    ap.add_argument("--out-dir", help="Where to write summary.{csv,json} (default: run-dir).")
    ap.add_argument("--tasks", help="Task JSONL (used by --validate).")
    ap.add_argument("--validate", action="store_true",
                    help="Validate task/label data and exit (needs --tasks and --labels).")
    args = ap.parse_args(argv)

    if args.validate:
        if not args.tasks or not args.labels:
            ap.error("--validate requires --tasks and --labels")
        return validate(args.tasks, args.labels)

    if not args.labels:
        ap.error("--labels is required")
    results_path = args.results
    if not results_path:
        if not args.run_dir:
            ap.error("provide --run-dir or --results")
        results_path = os.path.join(args.run_dir, "parsed", "results.jsonl")
    if not os.path.exists(results_path):
        raise SystemExit(f"results not found: {results_path}")

    out_dir = args.out_dir or args.run_dir or os.path.dirname(os.path.dirname(results_path))
    os.makedirs(out_dir, exist_ok=True)

    results = read_jsonl(results_path)
    labels_by_task = load_labels(args.labels)
    summary, scored, skipped = analyze(results, labels_by_task)

    summary_json = os.path.join(out_dir, "summary.json")
    summary_csv = os.path.join(out_dir, "summary.csv")
    with open(summary_json, "w", encoding="utf-8") as fh:
        json.dump(summary, fh, indent=2)
    write_csv(summary, summary_csv)

    print_console(summary)
    if skipped:
        uniq = sorted({t for t, _ in skipped})
        print(f"\nSkipped {len(uniq)} task(s) with no labels: {', '.join(uniq)}")
    print(f"\nWrote {summary_csv}")
    print(f"Wrote {summary_json}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
