# Agentgrep Evaluation Plan

This document defines how to evaluate Agentgrep against a baseline and future retrieval modes.

The goal is to measure usefulness on real coding tasks, not toy examples.

---

## Modes

### Mode A — rg baseline

Tool: `rg` only. No Agentgrep.

How an agent uses it:

```bash
rg "search term" --json
rg "search term" -l
```

What it provides:
- raw match lines grouped by file;
- no ranking;
- no symbol context;
- no graph context;
- no next actions;
- no JSON schema.

Baseline for: recall. If Agentgrep's top result is not in `rg`'s output, that is a bug.

### Mode B — agentgrep, no index, no semantic

Tool: `agentgrep find` with no index built.

```bash
agentgrep find "search term"
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

What it does not add (no index):
- symbol-name ranking boost;
- import/reference graph context;
- `map`, `symbol`, `related`, `blast` with full graph.

### Mode C — agentgrep, index, no semantic

Tool: `agentgrep` with `agentgrep index` built. No semantic/embedding layer.

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

This is the current production mode.

### Mode D — agentgrep, index + semantic (future)

Tool: `agentgrep` with `--semantic` flag (not yet implemented).

```bash
agentgrep index --semantic
agentgrep find --semantic "where is auth state restored"
```

What it adds over Mode C:
- embedding-based recall for conceptual queries;
- semantic candidates labeled separately from deterministic evidence.

Rules when implemented:
- disabled by default;
- explicit flag required;
- local-only embeddings;
- no always-running model;
- no GPU required;
- semantic results clearly distinguished from deterministic results.

---

## Optional future comparisons

These are not current work. Add only if real codebase tests show meaningful gap.

- **graphify**: graph-based navigation tool. Compare symbol/edge coverage.
- **SocratiCode**: LLM-augmented repo Q&A. Compare answer quality on reasoning tasks vs agentgrep evidence.
- **ctags / universal-ctags**: compare symbol extraction accuracy for `symbol` command.
- **tree-sitter CLI**: compare parse accuracy for symbol/reference extraction.

---

## Task types

Evaluate on real repos, not toy examples. Tasks should reflect actual coding agent work.

### Task type 1: Feature localization

> "Find the code that handles user authentication."

Measure:
- does the top `find` candidate contain the relevant code?
- how many files must an agent read before finding the right one?

### Task type 2: Symbol tracing

> "Find all places where `SearchResult` is created or consumed."

Measure:
- does `symbol SearchResult` find the definition?
- are production references separated from test references?
- are there false positives in unrelated files?

### Task type 3: Error message lookup

> "Where is the error 'rg was not found' generated?"

Measure:
- does `find "rg was not found"` return the right file as top candidate?
- is the line range correct?

### Task type 4: Change impact

> "I am editing `src/search.rs`. What else might break?"

Measure:
- does `blast src/search.rs` list the actually-impacted files?
- does it miss any important ones?
- does it list false positives?

### Task type 5: Refactor preparation

> "I want to rename `SearchResult` to `FindCandidate`."

Measure:
- does `symbol SearchResult` find all definition sites?
- does `related SearchResult` show the files that import or reference it?
- does `blast SearchResult` estimate the correct scope?

---

## Suggested real repos for evaluation

Start with repos where the maintainer can verify results:

1. **Agentgrep itself** — known, small, self-describing.
2. A medium-sized Rust CLI (5k–20k lines).
3. A TypeScript/Node project with multiple packages.
4. A Python project with imports and test files.
5. A mixed-language monorepo.

Avoid user-owned private repos in written eval results unless the owner consents.

---

## Metrics

For each task and mode, record:

| Metric | Description |
|---|---|
| Top-1 hit | Correct file is first candidate |
| Top-3 hit | Correct file is in top 3 |
| False positive rate | Unrelated files in top 5 |
| Evidence quality | `Why` / evidence labels are accurate |
| JSON stability | No schema breaks between runs |
| Latency | Time to complete command (approx) |
| Index freshness | Index was not stale during test |

Record failures specifically:
- wrong top result;
- missing symbol;
- noisy evidence;
- bad next action;
- incorrect risk level;
- stale index behavior;
- JSON contract break;
- slow command (>2s for small repo).

---

## Running an eval session

```bash
# 1. Build agentgrep
cargo install --path .

# 2. Navigate to target repo
cd /path/to/target/repo

# 3. Build index (Mode C)
agentgrep index

# 4. Run tasks, capture output
agentgrep find "feature term" --json > eval/find-feature.json
agentgrep symbol KnownSymbol --json > eval/symbol-known.json
agentgrep blast src/important.rs --json > eval/blast-important.json

# 5. Review results manually
# Record: correct? wrong? missing? noisy?
```

Capture output to `manual-test/<repo-name>/` for audit.

---

## Scope

This plan covers Modes A–C now.

Mode D is future work when the semantic layer is implemented.

The optional tool comparisons are lower priority and should only be done if real codebase evidence shows a specific gap.
