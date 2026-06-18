# Task & Data Schemas

This document defines the data formats used by the public retrieval benchmark.
Everything is newline-delimited JSON (JSONL): one JSON object per line, UTF-8, no
trailing commas, no comments inside the data files. Keep these files small and
diff-friendly.

All file paths in tasks and labels are **repo-relative, forward-slash** paths
(`crates/core/flags/parse.rs`, not `crates\core\...` and not absolute). This
matches the path form Agentgrep emits in `find --json` candidates, so labels and
results compare directly without normalization.

The three input files plus the harness output are:

| File | Schema | Produced by |
|---|---|---|
| `public-repos.jsonl` | Repo manifest | Maintained by hand |
| `tasks/<set>.jsonl` | Task definitions | Maintained by hand |
| `labels/<set>.jsonl` | Relevance labels | Maintained by hand (after inspecting the repo) |
| `eval-results/<run-id>/parsed/results.jsonl` | Mode output | `scripts/run-eval.ps1` |

---

## Repo manifest schema (`public-repos.jsonl`)

One object per repository under evaluation.

```json
{
  "repo_id": "ripgrep",
  "url": "https://github.com/BurntSushi/ripgrep",
  "commit": "13.0.0",
  "language": "rust",
  "approx_loc": 40000,
  "license": "MIT/Unlicense",
  "notes": "Self-contained Rust CLI. Good for symbol tracing and error lookup."
}
```

| Field | Type | Required | Meaning |
|---|---|---|---|
| `repo_id` | string | yes | Stable short id used as the worktree dir name and the join key in tasks/labels. `[a-z0-9-]`. |
| `url` | string | yes | Public clone URL (HTTPS). Must be a public repository. |
| `commit` | string | yes | **Pinned** commit SHA or tag. Required for reproducibility — never a branch name. |
| `language` | string | yes | Primary language (for grouping/reporting). |
| `approx_loc` | number | no | Rough size, for the small/medium/large bucket. |
| `license` | string | no | License identifier, recorded so re-runners know the terms. |
| `notes` | string | no | Why this repo is in the set; what it exercises. |

Prefer a tag (`13.0.0`) or full SHA over a short SHA. The harness checks out
exactly this ref, so it must resolve deterministically on a fresh clone.

---

## Task JSONL schema (`tasks/<set>.jsonl`)

One object per task. A task is a single retrieval question against one repo.

```json
{
  "task_id": "ripgrep-err-001",
  "repo_id": "ripgrep",
  "task_type": "exact-error-lookup",
  "query": "error parsing glob",
  "symbol": null,
  "target_path": null,
  "notes": "Known error string emitted during glob compilation."
}
```

| Field | Type | Required | Meaning |
|---|---|---|---|
| `task_id` | string | yes | Globally unique id. Convention: `<repo_id>-<type-abbrev>-NNN`. |
| `repo_id` | string | yes | Must match a `repo_id` in the manifest. |
| `task_type` | string | yes | One of the task types below. Used for per-type reporting. |
| `query` | string | yes | The natural-language or string query passed to `find` / `rg`. |
| `symbol` | string \| null | no | For symbol/refactor tasks: the symbol name (used for manual `symbol`/`related`/`blast` review). |
| `target_path` | string \| null | no | For impact tasks: the file the agent intends to edit (used for manual `blast` review). |
| `notes` | string | no | Context for reviewers; not used by the harness. |

### Task types

These mirror the categories in [TASKS.md](./TASKS.md):

- `feature-localization`
- `exact-error-lookup`
- `symbol-tracing`
- `impact-check`
- `refactor-prep`

The automated retrieval harness scores every task by running `find` (and `rg`
for Mode A) with `query` and comparing the ranked file list to the labels.
`symbol` and `target_path` support manual graph-quality review and future
command-specific scoring; they are not required for the core retrieval metrics.

---

## Label JSONL schema (`labels/<set>.jsonl`)

One object per task, giving the ground-truth relevant files. **Do not write
labels for a repo you have not actually inspected at the pinned commit.**

```json
{
  "task_id": "ripgrep-err-001",
  "repo_id": "ripgrep",
  "labels": [
    { "path": "crates/globset/src/glob.rs", "label": "primary" },
    { "path": "crates/globset/src/lib.rs", "label": "acceptable" },
    { "path": "crates/core/flags/hiargs.rs", "label": "supporting" },
    { "path": "crates/core/main.rs", "label": "irrelevant" }
  ]
}
```

| Field | Type | Required | Meaning |
|---|---|---|---|
| `task_id` | string | yes | Must match a task. |
| `repo_id` | string | yes | Must match the task's repo. |
| `labels` | array | yes | List of `{ path, label }` entries. |
| `labels[].path` | string | yes | Repo-relative forward-slash path at the pinned commit. |
| `labels[].label` | string | yes | One of the label types below. |

### Label types

| Label | Meaning |
|---|---|
| `primary` | The file that directly answers the task. There should usually be exactly one (occasionally a few). A correct top result is a `primary` file. |
| `acceptable` | A file that also genuinely answers the task — an equally valid landing spot. Counts as a hit. |
| `supporting` | Relevant context (a caller, a related module, a test) that helps but does not by itself answer the task. Counts toward Precision/Recall/nDCG, not Hit@k. |
| `irrelevant` | A file that looks plausible (matches the query string, similar name) but is **not** the answer. Used to measure noise and semantic bad-promotions. Optional but valuable. |

Files not listed are treated as unlabeled (gain 0, not relevant). You do not
need to enumerate the whole repo — label the answer(s), the useful context, and
any notable false-positive traps.

See [METRICS.md](./METRICS.md) for exactly how each label maps to gains and the
hit/relevant sets.

---

## Mode output schema (`eval-results/<run-id>/parsed/results.jsonl`)

`scripts/run-eval.ps1` writes one object per (task, mode) run. This is the input
to `scripts/analyze-eval.py`.

```json
{
  "run_id": "2026-06-18-1530",
  "task_id": "ripgrep-err-001",
  "repo_id": "ripgrep",
  "task_type": "exact-error-lookup",
  "mode": "C",
  "query": "error parsing glob",
  "command": "agentgrep find \"error parsing glob\" --json",
  "exit_code": 0,
  "latency_ms": 184,
  "json_parse_ok": true,
  "ranked_paths": [
    "crates/globset/src/glob.rs",
    "crates/globset/src/lib.rs"
  ],
  "semantic_status": "not_requested",
  "raw_stdout_path": "raw/ripgrep-err-001-C.out",
  "raw_stderr_path": "raw/ripgrep-err-001-C.err",
  "skipped": false,
  "skip_reason": null
}
```

| Field | Type | Meaning |
|---|---|---|
| `run_id` | string | Identifies the whole run; the output directory name. |
| `task_id` / `repo_id` / `task_type` | string | Copied from the task so the analyzer needs only results + labels. |
| `mode` | string | `A`, `B`, `C`, or `D`. |
| `query` | string | The query actually issued. |
| `command` | string | Human-readable command line, for audit. |
| `exit_code` | number | Process exit code. |
| `latency_ms` | number | Wall-clock time around the command, measured by the harness. |
| `json_parse_ok` | bool | Whether the captured stdout parsed as valid JSON (rg JSON-lines for A; find contract for B/C/D). |
| `ranked_paths` | string[] | Ranked candidate file paths, best first. For A: rg files ordered by match count desc. For B/C/D: `find` candidate order. |
| `semantic_status` | string | From `coverage.semantic_status` for Mode D (`active` / `not_requested`); null for A. |
| `raw_stdout_path` / `raw_stderr_path` | string | Paths (relative to the run dir) to the full captured streams. |
| `skipped` | bool | True if the mode was skipped (e.g. Mode D with no semantic index). |
| `skip_reason` | string \| null | Why it was skipped. Skipped modes are excluded from metrics. |

---

## Reproducibility requirements

Every published benchmark run must be reproducible by an unaffiliated person from
public artifacts alone. A run is reproducible only if all of the following are
recorded:

1. **Repo URL** — public HTTPS clone URL (in the manifest).
2. **Pinned commit** — exact SHA or tag, never a moving branch (in the manifest).
3. **Raw outputs** — full stdout and stderr for every (task, mode), stored under
   `eval-results/<run-id>/raw/`. Never summarize away the raw output.
4. **Exit code** — captured per command.
5. **Latency** — wall-clock ms per command, measured by the harness.
6. **Environment** — Agentgrep version (`agentgrep --version`), `rg --version`,
   OS, and whether Mode D ran, recorded in `eval-results/<run-id>/run-meta.json`.

If any of these is missing, the numbers are not a benchmark — they are an
anecdote. Do not publish them as benchmark claims.
