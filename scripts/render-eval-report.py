#!/usr/bin/env python3
"""Generate a static HTML + Markdown benchmark report from an Agentgrep eval run.

Reads (from --run-dir, all optional except summary.json):
  summary.json              metric tables grouped by mode / repo / task type
  summary.csv               same data in CSV (not consumed; noted for parity)
  parsed/results.jsonl      per-task records (enables detailed tables)
  run-meta.json             environment, Agentgrep version, modes used

Optional:
  --labels <path>           relevance label JSONL; enables win/regression/miss tables

Writes (to --out-dir, default <run-dir>/report):
  index.html                self-contained static HTML report
  report.md                 concise Markdown benchmark summary
  assets/hit_by_mode.svg
  assets/mrr_ndcg_by_mode.svg
  assets/latency_by_mode.svg
  assets/semantic_deltas.svg   (only when Mode D data is present)

No external network dependencies. Python 3.8+ stdlib only.

Usage:
  python scripts/render-eval-report.py --run-dir eval-results/<run-id>
  python scripts/render-eval-report.py --run-dir eval-results/<run-id> \\
      --labels docs/evaluation/labels/public-v0.1.jsonl
"""
from __future__ import annotations

import argparse
import html as _html_mod
import json
import math
import os
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Dict, List, Optional, Tuple

REPORT_VERSION = "0.1"
K = 8

# ── colour palette ─────────────────────────────────────────────────────────────

_MODE_COLOR: Dict[str, str] = {
    "A": "#6baed6", "B": "#74c476", "C": "#fd8d3c", "D": "#9e9ac8",
}
HIT1_CLR  = "#2171b5"
HIT3_CLR  = "#6baed6"
HIT8_CLR  = "#bdd7e7"
MRR_CLR   = "#e6550d"
NDCG_CLR  = "#31a354"
P50_CLR   = "#3182bd"
P95_CLR   = "#de2d26"
HELP_CLR  = "#41ab5d"
BADP_CLR  = "#d73027"
REGR_CLR  = "#f46d43"

GAIN = {"primary": 3.0, "acceptable": 2.0, "supporting": 1.0, "irrelevant": 0.0}
HIT_LABELS = {"primary", "acceptable"}
RELEVANT_LABELS = {"primary", "acceptable", "supporting"}


# ══════════════════════════════════════════════════════════════════════════════
# SVG chart engine (stdlib-only)
# ══════════════════════════════════════════════════════════════════════════════

def _esc(s: Any) -> str:
    return _html_mod.escape(str(s) if s is not None else "", quote=True)


def _nice_ticks(lo: float, hi: float, n: int = 5) -> List[float]:
    if hi <= lo or not math.isfinite(hi - lo):
        return [0.0, max(lo, hi, 1.0)]
    rng = hi - lo
    raw = rng / n
    if raw <= 0:
        return [lo, hi]
    mag = 10 ** math.floor(math.log10(raw))
    step = mag * min(
        (1, 2, 2.5, 5, 10), key=lambda m: abs(m * mag - raw)
    )
    start = math.floor(lo / step) * step
    ticks: List[float] = []
    t = start
    while t <= hi + step * 1e-6:
        ticks.append(round(t, 10))
        t = round(t + step, 10)
    return ticks


def _svg_grouped_bar(
    title: str,
    x_labels: List[str],
    series_names: List[str],
    series_values: List[List[Optional[float]]],
    colors: List[str],
    *,
    y_max: Optional[float] = None,
    y_label: str = "",
    ms_label: bool = False,
    width: int = 560,
    height: int = 265,
) -> str:
    """Return an SVG grouped bar chart as a string. No external dependencies."""
    ml, mr, mt, mb = 62, 18, 36, 56
    cw = width - ml - mr
    ch = height - mt - mb
    cx0, cy1 = ml, mt + ch

    all_vals = [
        v for sv in series_values
        for v in sv
        if v is not None and math.isfinite(v) and v >= 0
    ]
    raw_max = max(all_vals) if all_vals else 1.0
    vmax = max(y_max if y_max is not None else raw_max, 1e-9)

    ticks = _nice_ticks(0.0, vmax)
    axis_max = ticks[-1] if ticks else vmax
    if axis_max <= 0:
        axis_max = 1.0

    def vy(v: float) -> float:
        return cy1 - max(0.0, min(v / axis_max, 1.0)) * ch

    n_g = max(len(x_labels), 1)
    n_s = max(len(series_names), 1)
    gw = cw / n_g
    pad = max(gw * 0.12, 4.0)
    bw = (gw - 2 * pad) / n_s

    o: List[str] = []
    o.append(
        f'<svg xmlns="http://www.w3.org/2000/svg" '
        f'viewBox="0 0 {width} {height}" width="{width}" height="{height}">'
    )
    o.append(f'<rect width="{width}" height="{height}" fill="#f9f9f9" rx="4"/>')
    o.append(
        f'<text x="{width // 2}" y="24" text-anchor="middle" '
        f'font-family="system-ui,sans-serif" font-size="12.5" '
        f'font-weight="600" fill="#1a1a1a">{_esc(title)}</text>'
    )

    # gridlines and y-axis tick labels
    for tick in ticks:
        y = vy(tick)
        o.append(
            f'<line x1="{cx0}" y1="{y:.1f}" x2="{cx0 + cw}" y2="{y:.1f}" '
            f'stroke="#e0e0e0" stroke-width="1"/>'
        )
        if ms_label or axis_max >= 10:
            lab = f"{tick:.0f}"
        else:
            lab = f"{tick:.3f}"
        o.append(
            f'<text x="{cx0 - 5}" y="{y + 4:.1f}" text-anchor="end" '
            f'font-family="system-ui,sans-serif" font-size="9.5" fill="#666">{lab}</text>'
        )

    # axes
    o.append(
        f'<line x1="{cx0}" y1="{mt}" x2="{cx0}" y2="{cy1}" '
        f'stroke="#999" stroke-width="1.5"/>'
    )
    o.append(
        f'<line x1="{cx0}" y1="{cy1}" x2="{cx0 + cw}" y2="{cy1}" '
        f'stroke="#999" stroke-width="1.5"/>'
    )

    # y-axis label (rotated)
    if y_label:
        lx, ly = 11, mt + ch // 2
        o.append(
            f'<text x="{lx}" y="{ly}" text-anchor="middle" '
            f'font-family="system-ui,sans-serif" font-size="9.5" fill="#888" '
            f'transform="rotate(-90,{lx},{ly})">{_esc(y_label)}</text>'
        )

    # bars
    for gi, x_lbl in enumerate(x_labels):
        gx = cx0 + gi * gw + pad
        for si in range(n_s):
            val = series_values[si][gi] if gi < len(series_values[si]) else None
            color = colors[si % len(colors)] if colors else "#aaa"
            bx = gx + si * bw
            if val is not None and math.isfinite(val) and val >= 0:
                bh = max((val / axis_max) * ch, 0.0)
                by = cy1 - bh
                o.append(
                    f'<rect x="{bx:.1f}" y="{by:.1f}" '
                    f'width="{max(bw - 1.5, 1):.1f}" height="{max(bh, 1):.1f}" '
                    f'fill="{_esc(color)}" opacity="0.88" rx="2">'
                    f'<title>{_esc(series_names[si])}: {val:.4g}</title>'
                    f'</rect>'
                )
        # x-axis label
        lx = cx0 + (gi + 0.5) * gw
        o.append(
            f'<text x="{lx:.1f}" y="{cy1 + 15}" text-anchor="middle" '
            f'font-family="system-ui,sans-serif" font-size="10.5" fill="#333">'
            f'{_esc(x_lbl)}</text>'
        )

    # legend
    if series_names:
        leg_y = cy1 + 32
        item_w = cw / max(n_s, 1)
        for si, sname in enumerate(series_names):
            color = colors[si % len(colors)] if colors else "#aaa"
            lx = cx0 + si * item_w
            o.append(
                f'<rect x="{lx:.1f}" y="{leg_y}" width="9" height="9" '
                f'fill="{_esc(color)}" rx="2"/>'
            )
            o.append(
                f'<text x="{lx + 12:.1f}" y="{leg_y + 8.5}" '
                f'font-family="system-ui,sans-serif" font-size="9.5" fill="#444">'
                f'{_esc(sname)}</text>'
            )

    o.append('</svg>')
    return "\n".join(o)


# ══════════════════════════════════════════════════════════════════════════════
# Data loading
# ══════════════════════════════════════════════════════════════════════════════

def _norm_path(p: Any) -> Optional[str]:
    if p is None:
        return None
    s = str(p).replace("\\", "/")
    if s.startswith("./"):
        s = s[2:]
    return s or None


def _coerce_paths(value: Any) -> List[str]:
    if value is None:
        return []
    if isinstance(value, str):
        p = _norm_path(value)
        return [p] if p else []
    if isinstance(value, list):
        return [p for v in value for p in [_norm_path(v)] if p is not None]
    return []


def _read_jsonl(path: Path) -> List[Dict]:
    rows = []
    with open(path, encoding="utf-8-sig") as fh:
        for line in fh:
            line = line.strip()
            if line:
                try:
                    rows.append(json.loads(line))
                except json.JSONDecodeError:
                    pass
    return rows


def load_labels(path: Path) -> Dict[str, Dict[str, str]]:
    """Load label file; returns {task_id: {norm_path: label}}."""
    out: Dict[str, Dict[str, str]] = {}
    for row in _read_jsonl(path):
        tid = row.get("task_id")
        if not tid:
            continue
        labels: Dict[str, str] = {}
        for entry in row.get("labels") or []:
            p = _norm_path(entry.get("path"))
            lab = entry.get("label")
            if p and lab:
                labels[p] = lab
        out[tid] = labels
    return out


def load_data(run_dir: Path, labels_path: Optional[Path] = None) -> Dict[str, Any]:
    """Load all available eval outputs from run_dir."""
    summary_path = run_dir / "summary.json"
    if not summary_path.exists():
        raise SystemExit(
            f"summary.json not found in {run_dir}\n"
            "Run analyze-eval.py first to generate summary.json."
        )
    with open(summary_path, encoding="utf-8-sig") as fh:
        summary = json.load(fh)

    meta = None
    meta_path = run_dir / "run-meta.json"
    if meta_path.exists():
        with open(meta_path, encoding="utf-8-sig") as fh:
            meta = json.load(fh)

    raw_results: List[Dict] = []
    results_path = run_dir / "parsed" / "results.jsonl"
    if results_path.exists():
        raw_results = [r for r in _read_jsonl(results_path) if not r.get("skipped")]

    labels_by_task: Optional[Dict[str, Dict[str, str]]] = None
    if labels_path and labels_path.exists():
        labels_by_task = load_labels(labels_path)

    return {
        "summary": summary,
        "meta": meta,
        "raw_results": raw_results,
        "labels_by_task": labels_by_task,
        "run_dir": run_dir,
    }


# ══════════════════════════════════════════════════════════════════════════════
# Per-task metric computation (mirrors analyze-eval.py; used if labels supplied)
# ══════════════════════════════════════════════════════════════════════════════

def _first_hit_rank(ranked: List[str], labels: Dict[str, str]) -> Optional[int]:
    for i, p in enumerate(ranked, 1):
        if labels.get(p) in HIT_LABELS:
            return i
    return None


def _per_task_metrics(ranked: List[str], labels: Dict[str, str]) -> Dict[str, Any]:
    fhr = _first_hit_rank(ranked, labels)
    topk = ranked[:K]
    relevant = {p for p, l in labels.items() if l in RELEVANT_LABELS}
    n_rel = sum(1 for p in topk if p in relevant)
    gains = [GAIN.get(labels.get(p), 0.0) for p in topk]
    dcg = sum(g / math.log2(i + 1) for i, g in enumerate(gains, 1))
    ideal = sorted((GAIN.get(l, 0.0) for l in labels.values()), reverse=True)[:K]
    idcg = sum(g / math.log2(i + 1) for i, g in enumerate(ideal, 1))
    return {
        "hit@1": 1.0 if fhr and fhr <= 1 else 0.0,
        "hit@3": 1.0 if fhr and fhr <= 3 else 0.0,
        "hit@8": 1.0 if fhr and fhr <= 8 else 0.0,
        "rr": (1.0 / fhr) if fhr else 0.0,
        "ndcg@8": (dcg / idcg) if idcg > 0 else None,
        "miss": fhr is None,
        "irrelevant_promoted": {p for p in topk if labels.get(p) == "irrelevant"},
    }


def score_results(
    raw_results: List[Dict],
    labels_by_task: Dict[str, Dict[str, str]],
) -> List[Dict]:
    """Attach per-task hit metrics to raw results records (requires labels)."""
    scored = []
    for rec in raw_results:
        tid = rec.get("task_id") or ""
        labels = labels_by_task.get(tid)
        if not labels:
            continue
        ranked = _coerce_paths(rec.get("ranked_paths"))
        m = _per_task_metrics(ranked, labels)
        scored.append({
            **rec,
            "ranked": ranked,
            **m,
        })
    return scored


# ══════════════════════════════════════════════════════════════════════════════
# Analysis helpers
# ══════════════════════════════════════════════════════════════════════════════

def _index_by_task_mode(records: List[Dict]) -> Dict[str, Dict[str, Dict]]:
    """Index records as {task_id: {mode: record}}."""
    idx: Dict[str, Dict[str, Dict]] = {}
    for r in records:
        tid = r.get("task_id") or ""
        mode = r.get("mode") or ""
        idx.setdefault(tid, {})[mode] = r
    return idx


def get_semantic_wins(scored: List[Dict], max_n: int = 20) -> List[Dict]:
    """Tasks where Mode D found a hit but Mode C missed (top-8)."""
    cd = _index_by_task_mode(scored)
    out = []
    for tid, modes in cd.items():
        c, d = modes.get("C"), modes.get("D")
        if c and d and c.get("miss") and not d.get("miss"):
            out.append({
                "task_id": tid,
                "repo_id": c.get("repo_id", ""),
                "task_type": c.get("task_type", ""),
                "query": c.get("query", ""),
                "c_hit8": c.get("hit@8", 0.0),
                "d_hit8": d.get("hit@8", 0.0),
                "c_top3": (c.get("ranked") or _coerce_paths(c.get("ranked_paths")))[:3],
                "d_top3": (d.get("ranked") or _coerce_paths(d.get("ranked_paths")))[:3],
                "lat_delta_ms": (
                    (d.get("latency_ms") or 0) - (c.get("latency_ms") or 0)
                    if d.get("latency_ms") is not None and c.get("latency_ms") is not None
                    else None
                ),
            })
    out.sort(key=lambda r: (r["task_type"], r["task_id"]))
    return out[:max_n]


def get_semantic_regressions(scored: List[Dict], max_n: int = 20) -> List[Dict]:
    """Tasks where Mode C had Hit@1 but Mode D did not."""
    cd = _index_by_task_mode(scored)
    out = []
    for tid, modes in cd.items():
        c, d = modes.get("C"), modes.get("D")
        if c and d and c.get("hit@1") == 1.0 and d.get("hit@1") != 1.0:
            out.append({
                "task_id": tid,
                "repo_id": c.get("repo_id", ""),
                "task_type": c.get("task_type", ""),
                "query": c.get("query", ""),
                "c_hit1": c.get("hit@1", 0.0),
                "d_hit1": d.get("hit@1", 0.0),
                "c_top3": (c.get("ranked") or _coerce_paths(c.get("ranked_paths")))[:3],
                "d_top3": (d.get("ranked") or _coerce_paths(d.get("ranked_paths")))[:3],
                "lat_delta_ms": (
                    (d.get("latency_ms") or 0) - (c.get("latency_ms") or 0)
                    if d.get("latency_ms") is not None and c.get("latency_ms") is not None
                    else None
                ),
            })
    out.sort(key=lambda r: r["task_id"])
    return out[:max_n]


def get_bad_promotions(scored: List[Dict], max_n: int = 20) -> List[Dict]:
    """Tasks where Mode D promoted irrelevant files that Mode C did not surface."""
    cd = _index_by_task_mode(scored)
    out = []
    for tid, modes in cd.items():
        c, d = modes.get("C"), modes.get("D")
        if not (c and d):
            continue
        c_irr = c.get("irrelevant_promoted") or set()
        d_irr = d.get("irrelevant_promoted") or set()
        new_irr = d_irr - c_irr
        if new_irr:
            out.append({
                "task_id": tid,
                "repo_id": d.get("repo_id", ""),
                "task_type": d.get("task_type", ""),
                "query": d.get("query", ""),
                "new_irrelevant": sorted(new_irr),
            })
    out.sort(key=lambda r: (r["repo_id"], r["task_id"]))
    return out[:max_n]


def get_no_hit_tasks(scored: List[Dict], max_n: int = 30) -> List[Dict]:
    """Tasks where every mode failed to hit in the top 8."""
    by_task = _index_by_task_mode(scored)
    out = []
    for tid, modes in by_task.items():
        if all(r.get("miss") for r in modes.values()):
            first = next(iter(modes.values()))
            out.append({
                "task_id": tid,
                "repo_id": first.get("repo_id", ""),
                "task_type": first.get("task_type", ""),
                "query": first.get("query", ""),
                "modes_run": sorted(modes.keys()),
            })
    out.sort(key=lambda r: (r["repo_id"], r["task_id"]))
    return out[:max_n]


def get_slowest_queries(raw_results: List[Dict], max_n: int = 20) -> List[Dict]:
    rows = [
        {
            "task_id": r.get("task_id", ""),
            "repo_id": r.get("repo_id", ""),
            "mode": r.get("mode", ""),
            "query": r.get("query", ""),
            "latency_ms": r.get("latency_ms"),
            "json_parse_ok": r.get("json_parse_ok"),
        }
        for r in raw_results
        if r.get("latency_ms") is not None
    ]
    rows.sort(key=lambda r: -(r["latency_ms"] or 0))
    return rows[:max_n]


# ══════════════════════════════════════════════════════════════════════════════
# Ranking-diagnostics helpers (B/C/D lexical-winner analysis)
#
# Modes B, C, D all run through rank::rank_with_index (the same Rust function).
# B passes index=None; C passes the loaded lexical/symbol index; D is identical
# to C at this stage and then calls semantic::expand_candidates afterwards.
# expand_candidates can re-rank for non-identifier queries (score +=
# similarity*0.3 for existing candidates, new semantic-only files at
# similarity*0.8).  For identifier-like queries it only annotates; order is
# preserved.  There is no guard protecting high-confidence lexical winners.
# ══════════════════════════════════════════════════════════════════════════════

def _raw_top_evidence(run_dir: Optional[Path], raw_stdout_path: Optional[str]) -> str:
    """Best-effort: return score/confidence/evidence summary for top candidate in a raw .out file."""
    if not raw_stdout_path or run_dir is None:
        return "—"
    try:
        with open(run_dir / raw_stdout_path, encoding="utf-8-sig") as fh:
            data = json.load(fh)
        cands = data.get("candidates") or []
        if not cands:
            return "(empty)"
        c = cands[0]
        score = c.get("score")
        conf = c.get("confidence", "?")
        ev_types = list(dict.fromkeys(
            e.get("type", "") for e in (c.get("evidence") or []) if e.get("type")
        ))[:4]
        score_str = f"{score:.2f}" if isinstance(score, (int, float)) else "?"
        return f"score={score_str} conf={conf} ev=[{', '.join(ev_types) or 'none'}]"
    except Exception:
        return "—"


def _rank_of(path: Optional[str], ranked: List[str]) -> Optional[int]:
    """Return 1-based position of path in ranked list, or None if absent."""
    if not path:
        return None
    try:
        return ranked.index(path) + 1
    except ValueError:
        return None


def get_hit1_drop_tasks(
    scored: List[Dict],
    hi_mode: str,
    lo_mode: str,
    max_n: int = 30,
) -> List[str]:
    """Return sorted task IDs where hi_mode has Hit@1=1 but lo_mode does not (both modes present)."""
    by_task = _index_by_task_mode(scored)
    out = []
    for tid in sorted(by_task):
        modes = by_task[tid]
        hi = modes.get(hi_mode)
        lo = modes.get(lo_mode)
        if hi and lo and hi.get("hit@1") == 1.0 and lo.get("hit@1") != 1.0:
            out.append(tid)
    return out[:max_n]


def get_demotion_tasks(
    raw_results: List[Dict],
    base_mode: str,
    other_mode: str,
    max_n: int = 30,
) -> List[str]:
    """Return task IDs where base_mode rank-1 path appears in other_mode top-8 but is not rank-1.

    Does not require labels — purely a ranking-order comparison.
    """
    by_task = _index_by_task_mode(raw_results)
    out = []
    for tid in sorted(by_task):
        modes = by_task[tid]
        base = modes.get(base_mode)
        other = modes.get(other_mode)
        if not (base and other):
            continue
        b_paths = _coerce_paths(base.get("ranked_paths"))
        o_paths = _coerce_paths(other.get("ranked_paths"))
        b_top = b_paths[0] if b_paths else None
        if not b_top:
            continue
        rank = _rank_of(b_top, o_paths)
        if rank is not None and rank > 1:
            out.append(tid)
    return out[:max_n]


def build_diag_rows(
    task_ids: List[str],
    by_task_raw: Dict[str, Dict[str, Dict]],
    scored_by_task: Dict[str, Dict[str, Dict]],
    run_dir: Optional[Path],
) -> List[Dict]:
    """Build a standard diagnostic dict for each task_id with all B/C/D comparison columns."""
    rows = []
    for tid in task_ids:
        raw_modes = by_task_raw.get(tid, {})
        sc_modes = scored_by_task.get(tid, {})
        first = next(iter(raw_modes.values()), {})

        def ranked_for(mode: str) -> List[str]:
            r = sc_modes.get(mode) or raw_modes.get(mode)
            if not r:
                return []
            return _coerce_paths(r.get("ranked") or r.get("ranked_paths"))

        b, c, d = ranked_for("B"), ranked_for("C"), ranked_for("D")
        b_top = b[0] if b else None
        c_top = c[0] if c else None
        d_top = d[0] if d else None

        def ev(mode: str) -> str:
            r = raw_modes.get(mode)
            return _raw_top_evidence(run_dir, r.get("raw_stdout_path") if r else None)

        rows.append({
            "task_id": tid,
            "repo": first.get("repo_id", ""),
            "task_type": first.get("task_type", ""),
            "query": first.get("query", ""),
            "b_top": b_top,
            "c_top": c_top,
            "d_top": d_top,
            "b_top3": b[:3],
            "c_top3": c[:3],
            "d_top3": d[:3],
            "b_in_c8": (b_top in set(c[:8])) if b_top else None,
            "b_in_d8": (b_top in set(d[:8])) if b_top else None,
            "b_rank_in_c": _rank_of(b_top, c),
            "b_rank_in_d": _rank_of(b_top, d),
            "b_ev": ev("B"),
            "c_ev": ev("C"),
            "d_ev": ev("D"),
        })
    return rows


def _html_diag_table(rows: List[Dict], row_class: str = "loss") -> str:
    """Render the wide B/C/D diagnostic table as HTML."""
    if not rows:
        return '<p class="muted">None found.</p>'
    hdrs = [
        "Task ID", "Repo", "Type", "Query",
        "B top path", "C top path", "D top path",
        "B top-3", "C top-3", "D top-3",
        "B in C8?", "B in D8?",
        "B rank→C", "B rank→D",
        "B top evidence", "C top evidence", "D top evidence",
    ]

    def yn(v: Any) -> str:
        if v is None:
            return "—"
        return "yes" if v else '<span style="color:#c00">no</span>'

    def rk(v: Any) -> str:
        return str(v) if v is not None else "—"

    def mono(s: Any) -> str:
        return f'<span class="mono">{_h(s or "—")}</span>'

    table_rows = [
        [
            _h(r["task_id"]), _h(r["repo"]), _h(r["task_type"]),
            _h(r["query"])[:70],
            mono(r["b_top"]), mono(r["c_top"]), mono(r["d_top"]),
            _paths_cell(r["b_top3"]), _paths_cell(r["c_top3"]), _paths_cell(r["d_top3"]),
            yn(r["b_in_c8"]), yn(r["b_in_d8"]),
            rk(r["b_rank_in_c"]), rk(r["b_rank_in_d"]),
            f'<span class="mono" style="font-size:10px">{_h(r["b_ev"])}</span>',
            f'<span class="mono" style="font-size:10px">{_h(r["c_ev"])}</span>',
            f'<span class="mono" style="font-size:10px">{_h(r["d_ev"])}</span>',
        ]
        for r in rows
    ]
    return _html_table(hdrs, table_rows, row_classes=[row_class] * len(table_rows))


# ══════════════════════════════════════════════════════════════════════════════
# Chart generation
# ══════════════════════════════════════════════════════════════════════════════

def make_charts(summary: Dict, assets_dir: Path) -> Dict[str, str]:
    """Generate SVG chart files; return {slug: filename}."""
    assets_dir.mkdir(parents=True, exist_ok=True)
    charts: Dict[str, str] = {}
    by_mode = summary.get("by_mode") or {}
    modes = sorted(by_mode.keys())
    if not modes:
        return charts

    # Hit@1 / Hit@3 / Hit@8 by mode
    fname = "hit_by_mode.svg"
    svg = _svg_grouped_bar(
        "Hit Rate by Mode",
        modes,
        ["Hit@1", "Hit@3", "Hit@8"],
        [
            [by_mode[m].get("hit@1") for m in modes],
            [by_mode[m].get("hit@3") for m in modes],
            [by_mode[m].get("hit@8") for m in modes],
        ],
        [HIT1_CLR, HIT3_CLR, HIT8_CLR],
        y_max=1.0,
        y_label="Rate",
    )
    (assets_dir / fname).write_text(svg, encoding="utf-8")
    charts["hit"] = fname

    # MRR + nDCG@8 by mode
    fname = "mrr_ndcg_by_mode.svg"
    svg = _svg_grouped_bar(
        "MRR and nDCG@8 by Mode",
        modes,
        ["MRR", "nDCG@8"],
        [
            [by_mode[m].get("mrr") for m in modes],
            [by_mode[m].get("ndcg@8") for m in modes],
        ],
        [MRR_CLR, NDCG_CLR],
        y_max=1.0,
        y_label="Score",
    )
    (assets_dir / fname).write_text(svg, encoding="utf-8")
    charts["mrr_ndcg"] = fname

    # Latency p50 / p95 by mode
    fname = "latency_by_mode.svg"
    svg = _svg_grouped_bar(
        "Latency by Mode (ms)",
        modes,
        ["p50 ms", "p95 ms"],
        [
            [by_mode[m].get("latency_p50_ms") for m in modes],
            [by_mode[m].get("latency_p95_ms") for m in modes],
        ],
        [P50_CLR, P95_CLR],
        y_label="ms",
        ms_label=True,
    )
    (assets_dir / fname).write_text(svg, encoding="utf-8")
    charts["latency"] = fname

    # Semantic comparison rates (Mode D vs C)
    sem = (summary.get("semantic_c_vs_d") or {}).get("overall") or {}
    if sem.get("n", 0) > 0:
        fname = "semantic_deltas.svg"
        svg = _svg_grouped_bar(
            "Semantic Comparison: Mode D vs Mode C",
            ["Helpful Hit", "Bad Promotion", "Regression"],
            ["Rate"],
            [[
                sem.get("semantic_only_helpful_hit_rate"),
                sem.get("bad_promotion_rate"),
                sem.get("exact_query_regression_rate"),
            ]],
            [HELP_CLR, BADP_CLR, REGR_CLR],
            y_max=1.0,
            y_label="Rate",
        )
        (assets_dir / fname).write_text(svg, encoding="utf-8")
        charts["semantic"] = fname

    return charts


# ══════════════════════════════════════════════════════════════════════════════
# HTML helpers
# ══════════════════════════════════════════════════════════════════════════════

def _h(s: Any) -> str:
    return _html_mod.escape(str(s) if s is not None else "—")


def _frate(v: Any) -> str:
    if v is None:
        return "—"
    try:
        return f"{float(v):.3f}"
    except (ValueError, TypeError):
        return "—"


def _fms(v: Any) -> str:
    if v is None:
        return "—"
    try:
        return f"{float(v):.0f}"
    except (ValueError, TypeError):
        return "—"


def _fn(v: Any) -> str:
    if v is None:
        return "—"
    try:
        return str(int(v))
    except (ValueError, TypeError):
        return "—"


def _html_table(
    headers: List[str],
    rows: List[List[Any]],
    *,
    table_id: str = "",
    row_classes: Optional[List[str]] = None,
) -> str:
    id_attr = f' id="{_esc(table_id)}"' if table_id else ""
    lines = [f'<table{id_attr}>', "<thead><tr>"]
    for h in headers:
        lines.append(f"<th>{_h(h)}</th>")
    lines.append("</tr></thead><tbody>")
    for ri, row in enumerate(rows):
        cls = row_classes[ri] if row_classes and ri < len(row_classes) else ""
        cls_attr = f' class="{_esc(cls)}"' if cls else ""
        lines.append(f"<tr{cls_attr}>")
        for cell in row:
            lines.append(f"<td>{cell if cell is not None else '—'}</td>")
        lines.append("</tr>")
    lines.append("</tbody></table>")
    return "\n".join(lines)


def _tag(mode: str) -> str:
    return f'<span class="tag tag-{_esc(mode)}">{_esc(mode)}</span>'


def _paths_cell(paths: List[str], raw_path: Optional[str] = None) -> str:
    if not paths:
        return '<span class="muted">—</span>'
    items = [f'<span class="mono">{_h(p)}</span>' for p in paths]
    return "<br>".join(items)


_CSS = """\
:root {
  --bg: #fff; --fg: #1a1a1a; --muted: #555; --border: #ddd;
  --accent: #3b6fcc; --th-bg: #f2f4f7;
  --win-bg: #d4edda; --loss-bg: #f8d7da; --note-bg: #fff3cd;
}
* { box-sizing: border-box; }
body {
  font-family: system-ui, -apple-system, sans-serif; font-size: 14px;
  color: var(--fg); background: var(--bg);
  max-width: 1160px; margin: 0 auto; padding: 1.5rem 2rem;
}
h1 { font-size: 1.45rem; border-bottom: 2px solid var(--accent);
     padding-bottom: .4rem; margin-bottom: .75rem; }
h2 { font-size: 1.1rem; margin-top: 2.2rem;
     border-bottom: 1px solid var(--border); padding-bottom: .25rem; }
h3 { font-size: .95rem; margin-top: 1.4rem; color: #333; }
p { margin: .4rem 0; }
a { color: var(--accent); }
table { border-collapse: collapse; width: 100%; font-size: 12.5px; margin: .6rem 0; }
th { background: var(--th-bg); text-align: left;
     padding: .32rem .55rem; border: 1px solid var(--border); white-space: nowrap; }
td { padding: .28rem .55rem; border: 1px solid var(--border); vertical-align: top; }
tr:nth-child(even) td { background: #fafafa; }
.win td  { background: var(--win-bg)  !important; }
.loss td { background: var(--loss-bg) !important; }
.note { background: var(--note-bg); border: 1px solid #ffc107;
        border-radius: 4px; padding: .5rem .8rem; margin: .6rem 0;
        font-size: 13px; color: #665200; }
.mono { font-family: "Cascadia Code","Fira Code",monospace; font-size: 11px; }
.muted { color: var(--muted); font-size: 12px; }
.charts-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(265px, 1fr));
  gap: 1rem; margin: .8rem 0;
}
.chart-box {
  background: #fff; border: 1px solid var(--border);
  border-radius: 4px; padding: .5rem; text-align: center;
}
.chart-box img { max-width: 100%; height: auto; }
details { margin: .5rem 0; }
details > summary {
  cursor: pointer; font-weight: 600; padding: .35rem .6rem;
  background: var(--th-bg); border: 1px solid var(--border);
  border-radius: 3px; list-style: none; user-select: none;
}
details > summary::-webkit-details-marker { display: none; }
details > summary::before { content: "▶ "; font-size: .8em; }
details[open] > summary::before { content: "▼ "; }
details > div { padding: .5rem 0; overflow-x: auto; }
.tag { display: inline-block; padding: .1rem .35rem; border-radius: 3px;
       font-size: 11px; font-weight: 600; margin: 0 1px; }
.tag-A { background: #dbeeff; color: #1a5c8c; }
.tag-B { background: #d4f0db; color: #1a6b2e; }
.tag-C { background: #fde8d3; color: #8c4a00; }
.tag-D { background: #ede8fa; color: #4a2d8c; }
"""


# ══════════════════════════════════════════════════════════════════════════════
# HTML report assembly
# ══════════════════════════════════════════════════════════════════════════════

def make_html(
    summary: Dict,
    meta: Optional[Dict],
    raw_results: List[Dict],
    scored: Optional[List[Dict]],
    charts: Dict[str, str],
    run_dir: Path,
    out_dir: Path,
) -> str:
    by_mode = summary.get("by_mode") or {}
    modes = sorted(by_mode.keys())
    by_tt = summary.get("by_tasktype_mode") or {}
    by_repo = summary.get("by_repo_mode") or {}
    sem_ov = (summary.get("semantic_c_vs_d") or {}).get("overall") or {}
    sem_by_tt = (summary.get("semantic_c_vs_d") or {}).get("by_task_type") or {}
    has_d = "D" in modes or sem_ov.get("n", 0) > 0
    has_labels = scored is not None

    run_id = (meta or {}).get("run_id") or run_dir.name
    run_date = (meta or {}).get("timestamp_utc") or (meta or {}).get("date") or "unknown"
    ag_ver = (meta or {}).get("agentgrep_version") or "unknown"
    sem_enabled = (meta or {}).get("semantic_enabled")
    meta_task_file = (meta or {}).get("task_file") or ""
    meta_label_file = (meta or {}).get("label_file") or ""

    all_repos = sorted({r.get("repo_id", "") for r in raw_results if r.get("repo_id")})
    task_count = len({r.get("task_id") for r in raw_results})
    pair_count = len(raw_results)

    # Analysis tables (require labels)
    wins = get_semantic_wins(scored) if has_labels and has_d else []
    regressions = get_semantic_regressions(scored) if has_labels and has_d else []
    bad_promos = get_bad_promotions(scored) if has_labels and has_d else []
    no_hits = get_no_hit_tasks(scored) if has_labels else []
    slow = get_slowest_queries(raw_results)

    body: List[str] = []

    # ── header ────────────────────────────────────────────────────────────────
    body.append(
        f'<h1>Agentgrep Benchmark Report</h1>'
        f'<p class="muted">Generated '
        f'{datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M UTC")} '
        f'by render-eval-report v{REPORT_VERSION}</p>'
    )

    # ── run overview ──────────────────────────────────────────────────────────
    body.append('<h2>Run Overview</h2>')
    ov = [
        ("Run ID", _h(run_id)),
        ("Date (UTC)", _h(run_date)),
        ("Agentgrep version", _h(ag_ver)),
        ("Semantic enabled", _h("yes" if sem_enabled else "no")),
        ("Modes in summary", " ".join(_tag(m) for m in modes)),
        ("Repos evaluated",
         _h(", ".join(all_repos)) if all_repos else _h(len(by_repo))),
        ("Distinct tasks (raw)", _h(task_count or "—")),
        ("Raw records (excl. skipped)", _h(pair_count or "—")),
        ("Task file", f'<span class="mono">{_h(meta_task_file)}</span>' if meta_task_file else "—"),
        ("Label file", f'<span class="mono">{_h(meta_label_file)}</span>' if meta_label_file else "—"),
    ]
    ov_lines = ['<table style="max-width:560px"><tbody>']
    for label, val in ov:
        ov_lines.append(f'<tr><th style="width:190px">{_h(label)}</th><td>{val}</td></tr>')
    ov_lines.append('</tbody></table>')
    body.append("\n".join(ov_lines))

    # ── metric summary by mode ────────────────────────────────────────────────
    body.append('<h2>Metric Summary by Mode</h2>')
    m_hdrs = [
        "Mode", "n", "Hit@1", "Hit@3", "Hit@8", "MRR",
        "nDCG@8", "P@8", "R@8", "Misses", "JSON OK", "p50 ms", "p95 ms",
    ]
    m_rows = []
    for m in modes:
        mm = by_mode[m]
        m_rows.append([
            _tag(m), _fn(mm.get("n")),
            _frate(mm.get("hit@1")), _frate(mm.get("hit@3")), _frate(mm.get("hit@8")),
            _frate(mm.get("mrr")), _frate(mm.get("ndcg@8")),
            _frate(mm.get("precision@8")), _frate(mm.get("recall@8")),
            _fn(mm.get("misses")), _frate(mm.get("json_parse_success_rate")),
            _fms(mm.get("latency_p50_ms")), _fms(mm.get("latency_p95_ms")),
        ])
    body.append(_html_table(m_hdrs, m_rows))

    # ── metric summary by task type ───────────────────────────────────────────
    if by_tt:
        body.append('<h2>Metric Summary by Task Type</h2>')
        tt_hdrs = ["Task Type", "Mode", "n", "Hit@1", "Hit@3", "Hit@8",
                   "MRR", "nDCG@8", "p50 ms"]
        tt_rows: List[List[Any]] = []
        for tt in sorted(by_tt):
            tt_modes = sorted(by_tt[tt])
            for mi, m in enumerate(tt_modes):
                mm = by_tt[tt][m]
                tt_rows.append([
                    _h(tt) if mi == 0 else "",
                    _tag(m), _fn(mm.get("n")),
                    _frate(mm.get("hit@1")), _frate(mm.get("hit@3")), _frate(mm.get("hit@8")),
                    _frate(mm.get("mrr")), _frate(mm.get("ndcg@8")),
                    _fms(mm.get("latency_p50_ms")),
                ])
        body.append(_html_table(tt_hdrs, tt_rows))

    # ── metric summary by repo ────────────────────────────────────────────────
    if by_repo:
        body.append('<details><summary>Metric Summary by Repo</summary><div>')
        rp_hdrs = ["Repo", "Mode", "n", "Hit@1", "Hit@3", "Hit@8", "MRR", "nDCG@8", "p50 ms"]
        rp_rows: List[List[Any]] = []
        for repo in sorted(by_repo):
            repo_modes = sorted(by_repo[repo])
            for mi, m in enumerate(repo_modes):
                mm = by_repo[repo][m]
                rp_rows.append([
                    _h(repo) if mi == 0 else "",
                    _tag(m), _fn(mm.get("n")),
                    _frate(mm.get("hit@1")), _frate(mm.get("hit@3")), _frate(mm.get("hit@8")),
                    _frate(mm.get("mrr")), _frate(mm.get("ndcg@8")),
                    _fms(mm.get("latency_p50_ms")),
                ])
        body.append(_html_table(rp_hdrs, rp_rows))
        body.append('</div></details>')

    # ── charts ────────────────────────────────────────────────────────────────
    if charts:
        body.append('<h2>Charts</h2><div class="charts-grid">')
        chart_titles = {
            "hit": "Hit@1 / Hit@3 / Hit@8 by Mode",
            "mrr_ndcg": "MRR & nDCG@8 by Mode",
            "latency": "Latency (ms) by Mode",
            "semantic": "Semantic Deltas: Mode D vs Mode C",
        }
        for slug, fname in charts.items():
            title = chart_titles.get(slug, slug)
            body.append(
                f'<div class="chart-box">'
                f'<img src="assets/{_esc(fname)}" alt="{_esc(title)}" loading="lazy"/>'
                f'</div>'
            )
        body.append('</div>')

    # ── semantic analysis ─────────────────────────────────────────────────────
    if has_d:
        body.append('<h2>Semantic Analysis (Mode D vs Mode C)</h2>')
        if sem_ov.get("n", 0) > 0:
            s_hdrs = ["Metric", "Value", "Notes"]
            s_rows = [
                ["Paired tasks (C+D)", _fn(sem_ov.get("n")), "Tasks run in both modes"],
                [
                    "Semantic-only helpful hit rate",
                    _frate(sem_ov.get("semantic_only_helpful_hit_rate")),
                    "D top-8 hit where C top-8 missed — the upside case",
                ],
                [
                    "Bad promotion rate",
                    _frate(sem_ov.get("bad_promotion_rate")),
                    "Irrelevant file moved into D top-8 that wasn’t in C top-8",
                ],
                [
                    "Exact-query regression rate",
                    _frate(sem_ov.get("exact_query_regression_rate")),
                    "C had Hit@1, D did not — target: 0",
                ],
                [
                    "Latency delta p50 (ms)",
                    _fms(sem_ov.get("latency_delta_p50_ms")),
                    "Positive = D slower than C",
                ],
                [
                    "Latency delta p95 (ms)",
                    _fms(sem_ov.get("latency_delta_p95_ms")),
                    "",
                ],
            ]
            body.append(_html_table(s_hdrs, s_rows))

            if sem_by_tt:
                body.append('<h3>Semantic Comparison by Task Type</h3>')
                stt_hdrs = [
                    "Task Type", "n", "Helpful Hit", "Bad Promo",
                    "Regression", "Lat Δ p50 ms", "Lat Δ p95 ms",
                ]
                stt_rows = []
                for tt in sorted(sem_by_tt):
                    sm = sem_by_tt[tt]
                    stt_rows.append([
                        _h(tt), _fn(sm.get("n")),
                        _frate(sm.get("semantic_only_helpful_hit_rate")),
                        _frate(sm.get("bad_promotion_rate")),
                        _frate(sm.get("exact_query_regression_rate")),
                        _fms(sm.get("latency_delta_p50_ms")),
                        _fms(sm.get("latency_delta_p95_ms")),
                    ])
                body.append(_html_table(stt_hdrs, stt_rows))
        else:
            body.append(
                '<p class="muted">Mode D was present but no paired C+D data found in summary.json.</p>'
            )

    # ── analysis tables ───────────────────────────────────────────────────────
    body.append('<h2>Analysis Tables</h2>')

    if not has_labels:
        body.append(
            '<div class="note">Win/regression/miss tables require labels. '
            'Re-run with <code>--labels docs/evaluation/labels/public-v0.1.jsonl</code> '
            'to enable these tables.</div>'
        )
    else:
        # Detect discrepancies between summary.json (computed by analyze-eval.py)
        # and the label-derived tables (computed here). They can diverge if a
        # different labels file was used, or if summary.json was hand-written.
        warnings: List[str] = []
        # Check: per-mode miss count in summary vs per-(task,mode) miss count
        # from labels. Both count (task, mode) pairs where no hit appeared in
        # the top-8, so they must agree when the same labels are used.
        # Note: the "tasks with no useful top-8 hit" table is task-level (all
        # modes miss) and is intentionally a different concept; do not compare
        # it against the mode-level miss count.
        sum_misses = sum(
            (by_mode.get(m) or {}).get("misses") or 0 for m in modes
        )
        label_misses = sum(1 for r in scored if r.get("miss"))
        if sum_misses != label_misses:
            warnings.append(
                f"Miss count: summary.json reports {sum_misses} total mode-miss(es) "
                f"across all modes, but label-derived analysis finds {label_misses}. "
                "This usually means summary.json was generated with different labels "
                "or a different results.jsonl than those used here."
            )
        # Check: semantic helpful-hit sign vs wins table
        if has_d and sem_ov.get("n", 0) > 0:
            sum_hr = sem_ov.get("semantic_only_helpful_hit_rate") or 0.0
            label_wins = len(wins)
            if (sum_hr > 0) != (label_wins > 0):
                warnings.append(
                    f"Semantic wins: summary.json helpful-hit-rate={_frate(sum_hr)} "
                    f"({'> 0' if sum_hr > 0 else '= 0'}) but label-derived table "
                    f"found {label_wins} win(s). "
                    "Likely cause: summary.json and the current labels file differ."
                )
        if warnings:
            body.append(
                '<div class="note"><strong>Consistency note:</strong> '
                'The aggregate metrics in summary.json and the label-derived '
                'tables below do not fully agree. The tables reflect the labels '
                'file passed to this report; the metric tables above reflect the '
                'labels used when analyze-eval.py was run.<br>'
                + "<br>".join(f"• {_h(w)}" for w in warnings)
                + '</div>'
            )

    # Semantic wins
    if has_d:
        body.append('<h3>Best Semantic Wins (D found hit where C missed)</h3>')
        if wins:
            w_hdrs = ["Task ID", "Repo", "Type", "Query", "C top-3", "D top-3", "Lat Δ ms"]
            w_rows = [
                [
                    _h(r["task_id"]), _h(r["repo_id"]), _h(r["task_type"]),
                    _h(r["query"])[:80], _paths_cell(r["c_top3"]),
                    _paths_cell(r["d_top3"]), _fms(r.get("lat_delta_ms")),
                ]
                for r in wins
            ]
            body.append(_html_table(w_hdrs, w_rows, row_classes=["win"] * len(w_rows)))
        elif has_labels:
            body.append('<p class="muted">No wins found — Mode D never helped where C missed.</p>')

    # Semantic regressions
    if has_d:
        body.append('<h3>Worst Semantic Regressions (C had Hit@1, D did not)</h3>')
        if regressions:
            r_hdrs = ["Task ID", "Repo", "Type", "Query", "C top-3", "D top-3", "Lat Δ ms"]
            r_rows = [
                [
                    _h(r["task_id"]), _h(r["repo_id"]), _h(r["task_type"]),
                    _h(r["query"])[:80], _paths_cell(r["c_top3"]),
                    _paths_cell(r["d_top3"]), _fms(r.get("lat_delta_ms")),
                ]
                for r in regressions
            ]
            body.append(_html_table(r_hdrs, r_rows, row_classes=["loss"] * len(r_rows)))
        elif has_labels:
            body.append('<p class="muted">No exact-query regressions found.</p>')

    # Exact-query regressions (from summary — no per-task data needed)
    if has_d and sem_ov.get("exact_query_regression_rate", 0) and sem_ov["exact_query_regression_rate"] > 0:
        body.append(
            f'<p><strong>Exact-query regression rate: '
            f'{_frate(sem_ov.get("exact_query_regression_rate"))}</strong> '
            f'(target: 0.000 — any value here is a design concern)</p>'
        )

    # Bad promotions
    if has_d:
        body.append('<h3>Bad Promotions (irrelevant file surfaced by D, not by C)</h3>')
        if bad_promos:
            bp_hdrs = ["Task ID", "Repo", "Type", "Query", "Newly Promoted Irrelevant Paths"]
            bp_rows = [
                [
                    _h(r["task_id"]), _h(r["repo_id"]), _h(r["task_type"]),
                    _h(r["query"])[:80],
                    _paths_cell(r["new_irrelevant"]),
                ]
                for r in bad_promos
            ]
            body.append(_html_table(bp_hdrs, bp_rows, row_classes=["loss"] * len(bp_rows)))
        elif has_labels:
            body.append('<p class="muted">No bad promotions found.</p>')

    # Tasks with no useful top-8 hit
    body.append('<h3>Tasks with No Useful Top-8 Hit</h3>')
    if no_hits:
        nh_hdrs = ["Task ID", "Repo", "Type", "Query", "Modes Run"]
        nh_rows = [
            [
                _h(r["task_id"]), _h(r["repo_id"]), _h(r["task_type"]),
                _h(r["query"])[:80],
                " ".join(_tag(m) for m in r["modes_run"]),
            ]
            for r in no_hits
        ]
        body.append(_html_table(nh_hdrs, nh_rows))
    elif has_labels:
        body.append('<p class="muted">All tasks had at least one mode with a hit in top-8.</p>')

    # Slowest queries
    if slow:
        body.append(f'<h3>Slowest Queries (top {len(slow)})</h3>')
        sl_hdrs = ["Task ID", "Repo", "Mode", "Query", "Latency (ms)", "JSON OK"]
        sl_rows = [
            [
                _h(r["task_id"]), _h(r["repo_id"]), _tag(r["mode"]),
                _h(r.get("query", ""))[:70], _fms(r["latency_ms"]),
                _h("yes" if r.get("json_parse_ok") else "no"),
            ]
            for r in slow
        ]
        body.append(_html_table(sl_hdrs, sl_rows))

    # ── ranking diagnostics ───────────────────────────────────────────────────
    body.append('<h2>Ranking Diagnostics (B/C/D Lexical-Winner Analysis)</h2>')
    body.append(
        '<p>Modes B, C, and D all pass through the same '
        '<code>rank_with_index</code> lexical scoring function. '
        'Mode C additionally applies index evidence (symbol definitions, references, graph edges). '
        'Mode D applies the same index evidence and then re-sorts via '
        '<code>expand_candidates</code> (semantic cosine boost), which <strong>can demote '
        'a lexical rank-1 winner</strong> for non-identifier queries — no guard protects '
        'high-confidence lexical hits. For identifier-like queries (CamelCase / snake_case) '
        'semantic only annotates; deterministic order is preserved.</p>'
    )

    by_task_raw = _index_by_task_mode(raw_results)
    scored_by_task: Dict[str, Dict[str, Dict]] = (
        _index_by_task_mode(scored) if scored else {}
    )

    # ── hit@1 drop tables (require labels) ───────────────────────────────────
    if has_labels and scored:
        for hi_m, lo_m, label in [
            ("B", "C", "B Hit@1=1, C Hit@1=0"),
            ("B", "D", "B Hit@1=1, D Hit@1=0"),
            ("C", "D", "C Hit@1=1, D Hit@1=0 (semantic regression)"),
        ]:
            drop_ids = get_hit1_drop_tasks(scored, hi_m, lo_m)
            body.append(f'<h3>{_h(label)} — {len(drop_ids)} task(s)</h3>')
            if drop_ids:
                diag_rows = build_diag_rows(drop_ids, by_task_raw, scored_by_task, run_dir)
                body.append(_html_diag_table(diag_rows))
            else:
                body.append(
                    f'<p class="muted">No tasks where {hi_m} Hit@1=1 and {lo_m} Hit@1=0.</p>'
                )
    else:
        body.append(
            '<div class="note">Hit@1 drop tables require labels. '
            'Re-run with <code>--labels</code> to enable.</div>'
        )

    # ── path demotion tables (no labels required) ─────────────────────────────
    for base_m, other_m, label in [
        ("B", "C", "B rank-1 in C top-8 but demoted (index signals displaced lexical winner)"),
        ("B", "D", "B rank-1 in D top-8 but demoted (semantic or index displaced lexical winner)"),
    ]:
        dem_ids = get_demotion_tasks(raw_results, base_m, other_m)
        body.append(f'<h3>{_h(label)} — {len(dem_ids)} task(s)</h3>')
        if dem_ids:
            diag_rows = build_diag_rows(dem_ids, by_task_raw, scored_by_task, run_dir)
            body.append(_html_diag_table(diag_rows, row_class="note"))
        else:
            body.append(
                f'<p class="muted">No tasks where {base_m} rank-1 was demoted in {other_m}.</p>'
            )

    # ── per-task detail ───────────────────────────────────────────────────────
    if raw_results:
        by_task_mode = _index_by_task_mode(raw_results)
        detail_hdrs = (
            ["Task ID", "Repo", "Type", "Query"] +
            [f"{m}: Hit@1" for m in modes] +
            [f"{m}: top-3 paths" for m in modes] +
            ["Lat (ms)", "Modes"]
        )
        detail_rows: List[List[Any]] = []
        for tid in sorted(by_task_mode):
            tm = by_task_mode[tid]
            first = next(iter(tm.values()))
            row: List[Any] = [
                _h(tid), _h(first.get("repo_id", "")),
                _h(first.get("task_type", "")),
                _h(first.get("query", ""))[:70],
            ]
            # Hit@1 per mode (requires labels)
            for m in modes:
                mr = tm.get(m)
                if has_labels and scored:
                    # find scored record for this task/mode
                    scored_rec = next(
                        (s for s in scored
                         if s.get("task_id") == tid and s.get("mode") == m), None
                    )
                    row.append(_frate(scored_rec.get("hit@1") if scored_rec else None))
                else:
                    row.append("—" if not mr else '<span class="muted">needs labels</span>')
            # top-3 paths per mode
            for m in modes:
                mr = tm.get(m)
                if mr:
                    paths = _coerce_paths(mr.get("ranked_paths"))[:3]
                    raw_link = mr.get("raw_stdout_path") or ""
                    cell = _paths_cell(paths)
                    if raw_link:
                        cell += (
                            f'<br><a class="muted" href="{_esc(raw_link)}">[raw]</a>'
                        )
                    row.append(cell)
                else:
                    row.append("—")
            # latency (take mode C or first available)
            lat = None
            for m in ["C", "D", "B", "A"]:
                if m in tm and tm[m].get("latency_ms") is not None:
                    lat = tm[m]["latency_ms"]
                    break
            row.append(_fms(lat))
            row.append(" ".join(_tag(m) for m in sorted(tm)))
            detail_rows.append(row)

        body.append('<h2>Per-Task Detail</h2>')
        body.append(
            f'<details><summary>Per-task detail — {len(detail_rows)} task(s), '
            f'{len(modes)} mode(s)</summary>'
            '<div>' + _html_table(detail_hdrs, detail_rows) + '</div></details>'
        )

    # ── footer ────────────────────────────────────────────────────────────────
    body.append(
        '<hr style="margin-top:2rem;border:none;border-top:1px solid var(--border)">'
        f'<p class="muted">Agentgrep eval report · v{REPORT_VERSION} · '
        f'Run: {_h(run_id)} · '
        f'Source: <code>{_h(str(run_dir))}</code></p>'
    )

    title_str = f"Agentgrep Benchmark — {_esc(run_id)}"
    return (
        f"<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n"
        f"<meta charset=\"UTF-8\">\n"
        f"<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n"
        f"<title>{title_str}</title>\n"
        f"<style>\n{_CSS}\n</style>\n"
        f"</head>\n<body>\n"
        + "\n".join(body) +
        "\n</body>\n</html>\n"
    )


# ══════════════════════════════════════════════════════════════════════════════
# Markdown summary
# ══════════════════════════════════════════════════════════════════════════════

def make_markdown(
    summary: Dict,
    meta: Optional[Dict],
    raw_results: List[Dict],
    scored: Optional[List[Dict]],
    run_dir: Path,
    out_dir: Path,
) -> str:
    by_mode = summary.get("by_mode") or {}
    modes = sorted(by_mode.keys())
    sem_ov = (summary.get("semantic_c_vs_d") or {}).get("overall") or {}
    run_id = (meta or {}).get("run_id") or run_dir.name
    run_date = (meta or {}).get("timestamp_utc") or (meta or {}).get("date") or "unknown"
    ag_ver = (meta or {}).get("agentgrep_version") or "unknown"
    has_d = "D" in modes or sem_ov.get("n", 0) > 0
    has_labels = scored is not None

    def fr(v: Any) -> str:
        return f"{float(v):.3f}" if v is not None else "—"

    def fms(v: Any) -> str:
        return f"{float(v):.0f}" if v is not None else "—"

    def fn(v: Any) -> str:
        return str(int(v)) if v is not None else "—"

    lines: List[str] = []
    lines += [
        "# Agentgrep Benchmark Report\n",
        f"**Run:** `{run_id}`  **Date:** {run_date}  "
        f"**Version:** {ag_ver}\n",
    ]

    # Headline metrics
    lines += ["## Headline Metrics\n"]
    header = (
        "| Mode | n | Hit@1 | Hit@3 | Hit@8 | MRR | nDCG@8 | P@8 | R@8 | p50 ms | p95 ms |"
    )
    sep = "|---|---|---|---|---|---|---|---|---|---|---|"
    lines += [header, sep]
    for m in modes:
        mm = by_mode[m]
        lines.append(
            f"| {m} | {fn(mm.get('n'))} "
            f"| {fr(mm.get('hit@1'))} | {fr(mm.get('hit@3'))} | {fr(mm.get('hit@8'))} "
            f"| {fr(mm.get('mrr'))} | {fr(mm.get('ndcg@8'))} "
            f"| {fr(mm.get('precision@8'))} | {fr(mm.get('recall@8'))} "
            f"| {fms(mm.get('latency_p50_ms'))} | {fms(mm.get('latency_p95_ms'))} |"
        )
    lines.append("")

    # Semantic merge-gate
    if has_d and sem_ov.get("n", 0) > 0:
        n = sem_ov["n"]
        hr = sem_ov.get("semantic_only_helpful_hit_rate")
        bp = sem_ov.get("bad_promotion_rate")
        rr = sem_ov.get("exact_query_regression_rate")
        d50 = sem_ov.get("latency_delta_p50_ms")
        d95 = sem_ov.get("latency_delta_p95_ms")

        lines += ["## Semantic Merge-Gate Summary (Mode D vs Mode C)\n"]
        lines += [
            f"Paired tasks: **{n}**\n",
            "| Metric | Value |",
            "|--------|-------|",
            f"| Semantic-only helpful hit rate | {fr(hr)} |",
            f"| Bad promotion rate | {fr(bp)} |",
            f"| Exact-query regression rate | {fr(rr)} |",
            f"| Latency delta p50 (ms) | {fms(d50)} |",
            f"| Latency delta p95 (ms) | {fms(d95)} |",
            "",
        ]

        concerns = []
        if rr and rr > 0:
            concerns.append(
                f"- **Regression rate {fr(rr)} > 0** — exact-match demotion detected; "
                "target is 0; deterministic evidence should dominate ranking"
            )
        if bp and bp > 0.10:
            concerns.append(
                f"- **Bad promotion rate {fr(bp)} > 0.10** — semantic noise is high"
            )
        if hr is not None and hr < 0.05:
            concerns.append(
                f"- **Helpful hit rate {fr(hr)} < 0.05** — semantic adds little measurable value"
            )

        lines.append("### Gate Assessment\n")
        if concerns:
            lines.append("Concerns raised at current thresholds:\n")
            lines += concerns
        else:
            lines.append("No blocking concerns at current thresholds.")
        lines.append("")
    elif has_d:
        lines += ["## Semantic Analysis\n", "Mode D was included but no paired C+D data found.\n"]

    # Notable wins / regressions
    if has_labels and scored:
        wins = get_semantic_wins(scored, max_n=5)
        regs = get_semantic_regressions(scored, max_n=5)
        if wins:
            lines += ["## Notable Semantic Wins (top 5)\n"]
            for r in wins:
                lines.append(
                    f"- `{r['task_id']}` ({r['repo_id']}, {r['task_type']}): "
                    "C missed, D hit top-8"
                )
            lines.append("")
        if regs:
            lines += ["## Worst Semantic Regressions (top 5)\n"]
            for r in regs:
                lines.append(
                    f"- `{r['task_id']}` ({r['repo_id']}, {r['task_type']}): "
                    "C had Hit@1, D did not"
                )
            lines.append("")

    # Ranking diagnostics summary
    lines += ["## Ranking Diagnostics (B/C/D Lexical-Winner Analysis)\n"]
    lines += [
        "Modes B, C, and D all share the same `rank_with_index` lexical scoring function. "
        "C adds index evidence (symbols, edges). D adds semantic cosine re-ranking on top. "
        "For non-identifier queries, `expand_candidates` can demote a lexical rank-1 winner "
        "with no explicit guard protecting high-confidence lexical hits.\n",
    ]

    by_task_raw_md = _index_by_task_mode(raw_results)
    scored_by_task_md: Dict[str, Dict[str, Dict]] = (
        _index_by_task_mode(scored) if has_labels and scored else {}
    )

    lines.append("| Diagnostic | Count |")
    lines.append("|------------|-------|")
    if has_labels and scored:
        for hi_m, lo_m, label in [
            ("B", "C", "B Hit@1=1, C Hit@1=0"),
            ("B", "D", "B Hit@1=1, D Hit@1=0"),
            ("C", "D", "C Hit@1=1, D Hit@1=0"),
        ]:
            n = len(get_hit1_drop_tasks(scored, hi_m, lo_m))
            lines.append(f"| {label} | {n} |")
    else:
        lines.append("| Hit@1 drop tables | *requires --labels* |")

    for base_m, other_m, label in [
        ("B", "C", "B rank-1 demoted in C top-8"),
        ("B", "D", "B rank-1 demoted in D top-8"),
    ]:
        n = len(get_demotion_tasks(raw_results, base_m, other_m))
        lines.append(f"| {label} | {n} |")
    lines.append("")

    if has_labels and scored:
        # Show task IDs for each non-empty drop category
        for hi_m, lo_m, label in [
            ("B", "C", "B Hit@1=1, C Hit@1=0"),
            ("B", "D", "B Hit@1=1, D Hit@1=0"),
            ("C", "D", "C Hit@1=1, D Hit@1=0"),
        ]:
            drops = get_hit1_drop_tasks(scored, hi_m, lo_m, max_n=10)
            if drops:
                lines.append(f"### {label}\n")
                for tid in drops:
                    raw_m = by_task_raw_md.get(tid, {})
                    sc_m = scored_by_task_md.get(tid, {})
                    first = next(iter(raw_m.values()), {})
                    b_ranked = _coerce_paths((sc_m.get("B") or raw_m.get("B", {})).get("ranked") or
                                            (sc_m.get("B") or raw_m.get("B", {})).get("ranked_paths"))
                    c_ranked = _coerce_paths((sc_m.get("C") or raw_m.get("C", {})).get("ranked") or
                                            (sc_m.get("C") or raw_m.get("C", {})).get("ranked_paths"))
                    d_ranked = _coerce_paths((sc_m.get("D") or raw_m.get("D", {})).get("ranked") or
                                            (sc_m.get("D") or raw_m.get("D", {})).get("ranked_paths"))
                    b_top = b_ranked[0] if b_ranked else "—"
                    c_top = c_ranked[0] if c_ranked else "—"
                    d_top = d_ranked[0] if d_ranked else "—"
                    b_in_c = b_top in set(c_ranked[:8]) if b_ranked and c_ranked else False
                    b_rank_c = _rank_of(b_top if b_ranked else None, c_ranked)
                    lines.append(
                        f"- `{tid}` ({first.get('repo_id','')}): "
                        f"B→`{b_top}` C→`{c_top}` D→`{d_top}`"
                        + (f" | B in C top-8 at rank {b_rank_c}" if b_in_c and b_rank_c else "")
                    )
                lines.append("")

    # Demotion tasks
    for base_m, other_m, label in [
        ("B", "C", "B rank-1 demoted in C top-8"),
        ("B", "D", "B rank-1 demoted in D top-8"),
    ]:
        dems = get_demotion_tasks(raw_results, base_m, other_m, max_n=10)
        if dems:
            lines.append(f"### {label}\n")
            for tid in dems:
                raw_m = by_task_raw_md.get(tid, {})
                sc_m = scored_by_task_md.get(tid, {})
                first = next(iter(raw_m.values()), {})
                b_ranked = _coerce_paths(raw_m.get("B", {}).get("ranked_paths"))
                o_ranked = _coerce_paths(raw_m.get(other_m, {}).get("ranked_paths"))
                b_top = b_ranked[0] if b_ranked else "—"
                o_top = o_ranked[0] if o_ranked else "—"
                rank = _rank_of(b_top if b_ranked else None, o_ranked)
                lines.append(
                    f"- `{tid}` ({first.get('repo_id','')}): "
                    f"B rank-1=`{b_top}` → {other_m} rank-1=`{o_top}` "
                    f"(B path at {other_m} rank {rank})"
                )
            lines.append("")

    # Limitations
    lines += [
        "## Limitations\n",
        "- This benchmark measures *retrieval* (ranked file lists), "
        "not agentic task completion or edit success.",
        "- Results are valid only for the pinned repo commits used in this run.",
        "- Latency reflects this machine; never compare latency across machines.",
    ]
    if not has_d:
        lines.append("- Mode D (semantic) was not included in this run.")
    if not has_labels:
        lines.append(
            "- Win/regression/miss tables were not generated "
            "(rerun with `--labels` to enable)."
        )
    lines.append(
        "- Sample size may be small; check per-task counts before drawing conclusions."
    )
    lines.append("")

    # Output file paths — use relative paths so the report stays shareable
    # across machines. run_dir.name is the run-id directory (e.g. 2026-06-18-120000).
    rid = run_dir.name
    report_rel = f"{rid}/report"
    lines += [
        "## Output Files\n",
        "| File | Description |",
        "|------|-------------|",
        f"| `{rid}/summary.json` | Full metrics (machine-readable) |",
        f"| `{rid}/summary.csv` | Metrics CSV |",
        f"| `{rid}/parsed/results.jsonl` | Per-task raw results |",
        f"| `{rid}/run-meta.json` | Run environment |",
        f"| `{report_rel}/index.html` | HTML report |",
        f"| `{report_rel}/report.md` | This Markdown summary |",
        f"| `{report_rel}/assets/` | SVG chart files |",
        "",
        "To share: zip the `report/` directory, host `index.html` on any static file server,",
        "or use **File > Print > Save as PDF** in any browser for a PDF snapshot.",
        "",
    ]

    return "\n".join(lines)


# ══════════════════════════════════════════════════════════════════════════════
# Main
# ══════════════════════════════════════════════════════════════════════════════

def main(argv: Optional[List[str]] = None) -> int:
    ap = argparse.ArgumentParser(
        description=(
            "Generate a static HTML + Markdown benchmark report from an Agentgrep eval run.\n\n"
            "Reads summary.json (required), run-meta.json, and parsed/results.jsonl from\n"
            "--run-dir.  Writes index.html, report.md, and assets/*.svg to --out-dir.\n\n"
            "Pass --labels to enable per-task win/regression/miss analysis tables."
        ),
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    ap.add_argument(
        "--run-dir", required=True,
        help="Eval run directory (must contain summary.json).",
    )
    ap.add_argument(
        "--out-dir",
        help="Output directory (default: <run-dir>/report).",
    )
    ap.add_argument(
        "--labels",
        help="Relevance label JSONL (enables win/regression/miss tables).",
    )
    args = ap.parse_args(argv)

    run_dir = Path(args.run_dir).resolve()
    if not run_dir.is_dir():
        raise SystemExit(f"--run-dir not found: {run_dir}")

    out_dir = Path(args.out_dir).resolve() if args.out_dir else run_dir / "report"
    assets_dir = out_dir / "assets"

    labels_path: Optional[Path] = None
    if args.labels:
        labels_path = Path(args.labels).resolve()
        if not labels_path.exists():
            raise SystemExit(f"--labels file not found: {labels_path}")

    print(f"Loading data from: {run_dir}")
    data = load_data(run_dir, labels_path)
    summary = data["summary"]
    meta = data["meta"]
    raw_results = data["raw_results"]
    labels_by_task = data["labels_by_task"]

    scored: Optional[List[Dict]] = None
    if labels_by_task is not None and raw_results:
        scored = score_results(raw_results, labels_by_task)
        print(f"  Scored {len(scored)} records against labels")
    elif labels_path:
        print("  Warning: labels loaded but no results.jsonl found; analysis tables unavailable")

    print(f"Generating charts in: {assets_dir}")
    charts = make_charts(summary, assets_dir)
    for slug, fname in charts.items():
        size = (assets_dir / fname).stat().st_size
        print(f"  {fname}  ({size} B)")

    out_dir.mkdir(parents=True, exist_ok=True)

    print("Writing report...")
    html_content = make_html(summary, meta, raw_results, scored, charts, run_dir, out_dir)
    html_path = out_dir / "index.html"
    html_path.write_text(html_content, encoding="utf-8")
    print(f"  index.html  ({html_path.stat().st_size // 1024} KB)")

    md_content = make_markdown(summary, meta, raw_results, scored, run_dir, out_dir)
    md_path = out_dir / "report.md"
    md_path.write_text(md_content, encoding="utf-8")
    print(f"  report.md   ({md_path.stat().st_size // 1024} KB)")

    print(f"\nDone. Open: {html_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
