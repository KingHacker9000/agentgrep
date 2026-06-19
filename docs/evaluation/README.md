# Agentgrep Evaluation Scaffold

This folder contains the scaffold for comparing Agentgrep against a plain `rg` baseline and across its own modes.

**This is a scaffold, not benchmark claims.** No numbers have been collected yet. The purpose is to define what to measure before collecting results.

---

## Why evaluation before semantic mode

Semantic retrieval is tempting to add. But adding a layer before measuring the current layer means:

- no baseline to compare against;
- no way to know whether semantic actually helps;
- no way to know whether bugs are in the deterministic layer or the semantic layer.

The correct order:

1. Define what to measure.
2. Measure the deterministic local modes (A, B, C).
3. Identify where they fail on real tasks.
4. Only then, add semantic retrieval (Mode D) and measure whether it closes the gap.

---

## Comparison modes

### Mode A — rg baseline

Tool: `rg` only. No Agentgrep.

```bash
rg "search term" -l
rg "search term" --json
```

What it provides: raw line matches grouped by file. No ranking, no symbol context, no graph context, no JSON schema.

**Purpose:** recall floor. If Agentgrep's top result is not in `rg`'s output, that is a retrieval bug.

---

### Mode B — agentgrep, no index

Tool: `agentgrep find` without an index.

```bash
agentgrep find "search term" --json
```

What it adds over Mode A:

- file-level grouping and ranking;
- `--match any` / `--match all` multi-term support;
- `--role source` / `--include` / `--exclude` filters;
- line snippets attached to candidates;
- evidence labels (`Why` field);
- next-action suggestions;
- stable JSON output.

What it does not add without an index: symbol-name ranking boost, import/reference graph context, `map`, `symbol`, `related`, `blast` with full graph.

---

### Mode C — agentgrep, indexed (current production mode)

Tool: `agentgrep` with `agentgrep index` built.

```bash
agentgrep index
agentgrep find "search term" --json
agentgrep map <file> --json
agentgrep symbol <name> --json
agentgrep related <file-or-symbol> --json
agentgrep blast <file-or-symbol> --json
```

What it adds over Mode B:

- symbol-name expansion and ranking boost in `find`;
- BM25-style lexical scoring;
- graph-aware ranking;
- `map` with incoming/outgoing edges;
- `symbol` with definitions and references;
- `related` with import/reference edges;
- `blast` with graph-derived impact estimate.

---

### Mode D — agentgrep, indexed + semantic (active, experimental)

Semantic retrieval is **active** behind the `--semantic` flag. Provider: fastembed, model BAAI/bge-small-en-v1.5.

```bash
# Build semantic index (prompts for ~130 MB model download on first run)
agentgrep index --semantic
# or: agentgrep index --semantic --yes   (non-interactive)

# Semantic-expanded find
agentgrep find --semantic "where is auth state restored"
agentgrep find --semantic "SearchResult" --json
```

What it adds over Mode C:

- query is embedded and compared against pre-computed file vectors;
- semantic candidates are merged with deterministic candidates and labeled with `"semantic_match"` evidence;
- `coverage.semantic_status` is `"active"` when semantic contributed;
- deterministic evidence still dominates ranking.

Rules (unchanged from design):

- disabled by default (explicit `--semantic` required);
- local-only embeddings (no cloud API);
- no always-running model or daemon;
- no GPU required;
- semantic evidence labeled separately from deterministic evidence;
- default deterministic behavior unchanged.

**No formal evaluation results yet.** Mode D is experimental. Measure it against Mode C before drawing conclusions. See `docs/SEMANTIC.md` for limitations.

---

## Optional future comparisons

These are not current work. Add only if real codebase tests show a specific gap.

- **graphify** — graph-based navigation. Compare symbol/edge coverage.
- **SocratiCode** — LLM-augmented repo Q&A. Compare reasoning task answer quality.
- **ctags / universal-ctags** — compare symbol extraction accuracy.
- **tree-sitter CLI** — compare parse accuracy for symbol/reference extraction.

---

## Benchmark versions

### public-v0.1 — stable gated baseline (14 labeled tasks)

`tasks/public-v0.1.jsonl` and `labels/public-v0.1.jsonl` are **frozen**. The
regression gates in `scripts/check-eval-gates.py` are calibrated against this
set. Do not add or remove tasks here; gate thresholds are tied to the
per-task score distribution.

### public-v0.2 — expanded harder benchmark (26 labeled tasks, diagnostic)

`tasks/public-v0.2.jsonl` and `labels/public-v0.2.jsonl` add 12 harder tasks
on top of the v0.1 set (new symbol-tracing, refactor-prep, impact-check, and
workflow queries). This version is **diagnostic only**: no gates are enforced
against it yet. Use it to observe where the current modes fall short before
deciding on new gate thresholds.

---

## Contents of this folder

| File | Purpose |
|---|---|
| `README.md` | This file — scaffold overview and mode definitions |
| `BENCHMARKS.md` | Public benchmark philosophy, repo criteria, how to extend and rerun |
| `METRICS.md` | Metric definitions (automated + manual) and what not to overclaim |
| `REPORTING.md` | How to generate, share, and extend the static HTML/Markdown report |
| `TASK_SCHEMA.md` | Repo manifest, task, label, and mode-output schemas |
| `TASKS.md` | Task categories and example prompts |
| `RESULT_TEMPLATE.md` | Copy-paste template for recording one manual evaluation run |
| `public-repos.jsonl` | Repo manifest (pinned public repos) |
| `tasks/public-v0.1.jsonl` | Public task set — **frozen**, 14 labeled tasks, gated |
| `labels/public-v0.1.jsonl` | Relevance labels for the public task set — **frozen** |
| `tasks/public-v0.2.jsonl` | Expanded task set — 26 labeled tasks, diagnostic only |
| `labels/public-v0.2.jsonl` | Relevance labels for the expanded task set |

The runnable harness lives in `scripts/`:

| Script | Purpose |
|---|---|
| `scripts/run-eval.ps1` | Clone pinned repos, run Modes A–D, capture raw output + latency |
| `scripts/analyze-eval.py` | Compute metrics, write `summary.csv` / `summary.json` |
| `scripts/render-eval-report.py` | Generate static HTML + Markdown report from eval outputs |
| `scripts/check-eval-gates.py` | Enforce regression thresholds against `summary.json`; exits non-zero on failure |

---

## Public benchmark workflow (automated)

The automated benchmark runs the four modes against pinned public repos and
labeled tasks, then computes the retrieval and semantic metrics. This is the
path that produces reportable numbers. The manual session below is for
qualitative review.

```powershell
# 1. (Optional) validate task/label data first.
#    Use public-v0.1 for gated runs; public-v0.2 for diagnostic/exploratory runs.
python scripts/analyze-eval.py --validate `
  --tasks docs/evaluation/tasks/public-v0.1.jsonl `
  --labels docs/evaluation/labels/public-v0.1.jsonl

# 2. Run Modes A, B, C (and D only with -EnableSemantic).
powershell -ExecutionPolicy Bypass -File scripts/run-eval.ps1 `
  -RepoManifest docs/evaluation/public-repos.jsonl `
  -TaskFile     docs/evaluation/tasks/public-v0.1.jsonl `
  -LabelFile    docs/evaluation/labels/public-v0.1.jsonl `
  -OutDir       eval-results

# 3. Compute metrics for the run that just completed.
python scripts/analyze-eval.py `
  --run-dir eval-results/<run-id> `
  --labels  docs/evaluation/labels/public-v0.1.jsonl

# 4. Generate the static HTML + Markdown report.
python scripts/render-eval-report.py `
  --run-dir eval-results/<run-id> `
  --labels  docs/evaluation/labels/public-v0.1.jsonl
# Open eval-results/<run-id>/report/index.html in a browser.

# 5. Check regression gates (exits non-zero if any threshold is missed).
python scripts/check-eval-gates.py --run-dir eval-results/<run-id>
```

`run-eval.ps1 -Help` prints full options. Mode D is skipped unless
`-EnableSemantic` is passed and a semantic index can be built.

Outputs land under `eval-results/<run-id>/`: `raw/` (full stdout/stderr per
run), `parsed/results.jsonl`, `run-meta.json`, and `summary.{csv,json}`.
The report generator adds `report/index.html`, `report/report.md`, and
`report/assets/*.svg`. `eval-worktree/` and `eval-results/` are git-ignored.

See [BENCHMARKS.md](./BENCHMARKS.md) for philosophy and how to add a repo/task/
label, [TASK_SCHEMA.md](./TASK_SCHEMA.md) for the data formats, and
[METRICS.md](./METRICS.md) for metric definitions.

> **Agentic workflow evaluation is later work.** This benchmark measures
> *retrieval* — ranked file lists per mode. It does not measure multi-step agent
> loops, edit success, or end-to-end task completion. That is out of scope here.

---

## How to run an evaluation session

```bash
# 1. Build agentgrep
cargo install --path .

# 2. Navigate to target repo
cd /path/to/target/repo

# 3. Build index (Mode C)
agentgrep index

# 4. Run tasks, capture output
agentgrep find "feature term" --json > eval-find.json
agentgrep symbol KnownSymbol --json > eval-symbol.json
agentgrep blast src/important.rs --json > eval-blast.json

# 5. Record results
# Copy RESULT_TEMPLATE.md, fill it in, save to manual-test/<repo-name>/
```

See [TASKS.md](./TASKS.md) for task prompts. See [METRICS.md](./METRICS.md) for scoring guidance. See [RESULT_TEMPLATE.md](./RESULT_TEMPLATE.md) for the recording template.

---

## What still needs to happen before real benchmark claims

- At least 2 real repos evaluated with recorded results.
- Failure categories identified from real runs.
- Mode A (rg) baseline measured on the same tasks and repos.
- Results written to `manual-test/` and reviewed for accuracy before publishing.
