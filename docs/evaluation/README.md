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

### Mode D — agentgrep, indexed + semantic (future placeholder)

**Not yet implemented. Do not present as available.**

Planned shape when implemented:

```bash
agentgrep index --semantic
agentgrep find --semantic "where is auth state restored"
```

Rules when it exists:

- disabled by default;
- explicit flag required;
- local-only embeddings;
- no always-running model;
- no GPU required;
- semantic results labeled separately from deterministic results.

---

## Optional future comparisons

These are not current work. Add only if real codebase tests show a specific gap.

- **graphify** — graph-based navigation. Compare symbol/edge coverage.
- **SocratiCode** — LLM-augmented repo Q&A. Compare reasoning task answer quality.
- **ctags / universal-ctags** — compare symbol extraction accuracy.
- **tree-sitter CLI** — compare parse accuracy for symbol/reference extraction.

---

## Contents of this folder

| File | Purpose |
|---|---|
| `README.md` | This file — scaffold overview and mode definitions |
| `TASKS.md` | Task categories and example prompts |
| `METRICS.md` | Metric definitions and what not to overclaim |
| `RESULT_TEMPLATE.md` | Copy-paste template for recording one evaluation run |

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
