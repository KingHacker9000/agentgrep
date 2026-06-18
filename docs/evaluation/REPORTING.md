# Agentgrep Evaluation Report Generator

This document explains how to generate a static benchmark report from an eval
run, what the report files contain, and how to publish or share the result.

---

## Quick start

After running `analyze-eval.py` on a completed run, generate the report:

```powershell
# Minimal — uses summary.json only (no per-task analysis tables)
python scripts/render-eval-report.py --run-dir eval-results/<run-id>

# Full — enables win/regression/miss tables
python scripts/render-eval-report.py `
  --run-dir eval-results/<run-id> `
  --labels  docs/evaluation/labels/public-v0.1.jsonl

# Custom output directory
python scripts/render-eval-report.py `
  --run-dir eval-results/<run-id> `
  --labels  docs/evaluation/labels/public-v0.1.jsonl `
  --out-dir path/to/my-report
```

Open `eval-results/<run-id>/report/index.html` in any browser to view the report.

---

## Prerequisites

- Python 3.8 or later (stdlib only — no `pip install` required)
- A completed eval run: `eval-results/<run-id>/summary.json` must exist

`summary.json` is written by `scripts/analyze-eval.py`. Run that first if you
have only the raw outputs from `run-eval.ps1`:

```powershell
python scripts/analyze-eval.py `
  --run-dir eval-results/<run-id> `
  --labels  docs/evaluation/labels/public-v0.1.jsonl
```

---

## Report files

The generator writes the following files to `<run-dir>/report/` (or `--out-dir`):

| File | Description |
|------|-------------|
| `index.html` | Full static HTML report — self-contained, no CDN |
| `report.md` | Concise Markdown benchmark summary |
| `assets/hit_by_mode.svg` | Hit@1 / Hit@3 / Hit@8 grouped bar chart |
| `assets/mrr_ndcg_by_mode.svg` | MRR and nDCG@8 bar chart |
| `assets/latency_by_mode.svg` | p50 / p95 latency bar chart |
| `assets/semantic_deltas.svg` | Semantic win / bad promo / regression rates (Mode D only) |

The HTML report embeds CSS inline and references the SVG files by relative path.
The whole `report/` directory is self-contained: zipping it preserves the report
as-is.

### What the HTML report includes

- **Run overview**: run ID, date, Agentgrep version, modes, repos, task counts
- **Metric summary by mode**: Hit@1/3/8, MRR, nDCG@8, Precision@8, Recall@8,
  misses, JSON parse success rate, p50/p95 latency
- **Metric summary by task type**: same metrics broken out by `task_type`
- **Metric summary by repo** (collapsed): per-repo breakdown
- **Charts**: SVG bar charts for hit rates, MRR/nDCG, latency, semantic deltas
- **Semantic analysis** (if Mode D ran): paired C+D stats with gate assessment
- **Analysis tables** (require `--labels`):
  - Best semantic wins — tasks where D found a hit and C missed
  - Worst semantic regressions — tasks where C had Hit@1 and D did not
  - Bad promotions — irrelevant files surfaced by D but not by C
  - Tasks with no useful top-8 hit in any mode
  - Slowest queries
- **Per-task detail table**: task ID, repo, type, query, per-mode top-3 paths,
  latency, links to raw output files

### What the Markdown report includes

- Run metadata (ID, date, version)
- Headline metric table (all modes, all key metrics)
- Semantic merge-gate summary with a threshold-based gate assessment
- Notable wins and regressions (top 5 each, if labels provided)
- Limitations section
- Paths to all output files

---

## Inputs read by the generator

All inputs are read from `--run-dir`. All are optional except `summary.json`.

| File | Required | Source |
|------|----------|--------|
| `summary.json` | **Yes** | Written by `analyze-eval.py` |
| `summary.csv` | No | Written by `analyze-eval.py` (not consumed by reporter) |
| `parsed/results.jsonl` | No | Written by `run-eval.ps1` — enables per-task path display |
| `run-meta.json` | No | Written by `run-eval.ps1` — adds run overview metadata |
| Label JSONL (`--labels`) | No | Enables win/regression/miss analysis tables |

---

## Analysis tables and why they need labels

The win, regression, bad-promotion, and no-hit tables require computing per-task
hit metrics (Hit@1, Hit@8, miss), which requires knowing which files are labeled
`primary` or `acceptable` for each task. That information lives in the label
file, not in `summary.json`.

If you skip `--labels`, the aggregate metric tables and charts still work fully.
The analysis tables display a note explaining how to enable them.

---

## How to share or publish the report

### Static file host

The `report/` directory is fully self-contained. Copy it to any static host:

```bash
# GitHub Pages, Netlify, S3, etc.
cp -r eval-results/<run-id>/report/ public/benchmark-report/
```

No server-side code required. The HTML and SVG files load entirely from disk.

### Direct file share

Zip the `report/` directory and share the archive. Recipients open
`index.html` directly in their browser — no server needed.

```powershell
Compress-Archive -Path eval-results\<run-id>\report -DestinationPath agentgrep-report-<run-id>.zip
```

### PDF snapshot

Use your browser's **File > Print > Save as PDF** (or **Ctrl+P** → PDF) with
the HTML report open. This produces a reasonable single-file snapshot for
asynchronous review. It is not automatically generated — this is intentional to
avoid a headless browser dependency.

### What to commit

Do **not** commit `eval-results/` to the repository — it is git-ignored. If you
want a permanent record of a run:

- Commit `summary.json` and `summary.csv` to a `benchmark-results/<run-id>/`
  directory in the repo (small, human-readable).
- Attach the zipped `report/` to a GitHub release or PR as an artifact.

---

## Updating or extending the report

The generator is a single script with no external dependencies:
`scripts/render-eval-report.py`. SVG charts are built from scratch using the
Python stdlib. To add a chart or table:

1. Add a chart-generation call in `make_charts()`.
2. Reference it in the `make_html()` charts section.
3. Run `python scripts/render-eval-report.py --help` to confirm the script still
   loads correctly.

The SVG renderer (`_svg_grouped_bar`) supports grouped bar charts. For other
chart types, extend the SVG generation functions in the script.

---

## Verification

```powershell
# Confirm the script loads and prints help (no run required)
python scripts/render-eval-report.py --help

# Generate a report from an existing run
python scripts/render-eval-report.py `
  --run-dir eval-results/<run-id> `
  --labels  docs/evaluation/labels/public-v0.1.jsonl

# Check output
ls eval-results/<run-id>/report/
ls eval-results/<run-id>/report/assets/
```
