# Public Benchmarks

This document describes the **public** retrieval benchmark for Agentgrep: how it
is designed, how to extend it, and how to reproduce results. It is the companion
to [METRICS.md](./METRICS.md) (what is measured) and
[TASK_SCHEMA.md](./TASK_SCHEMA.md) (the data formats).

---

## Philosophy

The benchmark exists to answer one question honestly: **does Agentgrep help an
agent find the right file faster than plain `rg`, and does each layer (index,
semantic) earn its cost?**

To be credible, the benchmark must be:

- **Public.** Every repo, commit, task, and label lives in this repository or is
  fetched from a public URL. Anyone can clone, run, and check the numbers.
- **Reproducible.** Repos are pinned to exact commits. The harness records raw
  outputs, exit codes, and latency. Re-running on the same inputs yields the same
  retrieval results (semantic latency will vary; rankings should not).
- **Honest about scope.** We report per task type and per repo, never a single
  hero number. We mark example/placeholder data clearly and never invent labels.
- **Layer-attributable.** Modes A→D isolate each addition, so a win or regression
  can be traced to the layer that caused it.

This benchmark measures *retrieval* (ranked file lists). It does **not** measure
agentic workflow automation — multi-step agent loops, edit success, or
end-to-end task completion. That is explicitly later work
(see [README.md](./README.md)).

---

## No private project references

This benchmark is public and must stay that way.

- **No private or personal repositories.** Only public, independently-hosted
  repositories may be added.
- **No user-specific or local filesystem paths** in any committed file. The
  harness clones into a relative `eval-worktree/` directory under the repo.
- **No references to private projects of the author or maintainers.** Benchmark
  data must be meaningful to an outside reader with no inside knowledge.

If a task or label only makes sense to someone with private context, it does not
belong here.

---

## Criteria for choosing public repos

A good benchmark repo is:

1. **Public and permissively licensed** — clonable by anyone; license recorded in
   the manifest.
2. **Pinned-stable** — has tags or stable commits to pin to. We never track a
   moving branch.
3. **Real but inspectable** — large enough to be a genuine retrieval problem, but
   structured enough that a reviewer can verify the correct answer at the pinned
   commit.
4. **Diverse** — the set as a whole spans languages (Rust, TypeScript/Node,
   Python, mixed) and sizes (small <5k LOC, medium 5k–50k, large 50k+), because
   metrics must be reported per repo and we want coverage.
5. **Self-contained enough to index** — builds an Agentgrep index without exotic
   toolchains. (The benchmark indexes; it does not compile the target repo.)
6. **Stable answers** — the file that answers a task should not be a moving
   target between minor refactors near the pinned commit.

Avoid: tiny toy repos (no retrieval challenge), repos that are mostly generated
code or vendored dependencies, and repos whose "right answer" is genuinely
ambiguous.

---

## How to add a new repo, task, or label

The three data files are independent JSONL files. Add to each in order.

### 1. Add the repo

Append one line to [`public-repos.jsonl`](./public-repos.jsonl) following the
repo manifest schema. Use a public URL and a **pinned commit or tag**.

```json
{"repo_id":"myrepo","url":"https://github.com/owner/myrepo","commit":"v1.2.3","language":"python","approx_loc":12000,"license":"Apache-2.0","notes":"why it's here"}
```

### 2. Clone and inspect at the pinned commit

You cannot write correct labels without looking. Either run the harness once to
populate `eval-worktree/`, or clone manually:

```bash
git clone https://github.com/owner/myrepo eval-worktree/myrepo
git -C eval-worktree/myrepo checkout v1.2.3
```

### 3. Add tasks

Append lines to a task set file, e.g.
[`tasks/public-v0.1.jsonl`](./tasks/public-v0.1.jsonl). Each task references the
`repo_id` and has a `task_type` and a `query`. See
[TASK_SCHEMA.md](./TASK_SCHEMA.md#task-jsonl-schema-tasksetjsonl).

### 4. Add labels — only after inspecting

Append lines to the matching label set file, e.g.
[`labels/public-v0.1.jsonl`](./labels/public-v0.1.jsonl). Mark each relevant file
as `primary`, `acceptable`, `supporting`, or `irrelevant`, using repo-relative
paths verified at the pinned commit.

**Do not invent labels for a repo you have not inspected.** An unlabeled task is
better than a wrongly-labeled one — the analyzer simply skips tasks with no
labels. Clearly-marked `example`/`TODO` task ids are fine as templates; they
should not be counted as real results.

### 5. Sanity-check the data

```bash
python scripts/analyze-eval.py --validate \
  --tasks docs/evaluation/tasks/public-v0.1.jsonl \
  --labels docs/evaluation/labels/public-v0.1.jsonl
```

This checks that task/label ids cross-reference, label types are valid, and paths
are repo-relative — before you spend time on a full run.

---

## How to rerun results

The full loop is two commands: run, then analyze.

```powershell
# 1. Run all modes against all repos/tasks. Mode D (semantic) is skipped
#    unless -EnableSemantic is passed and a semantic index can be built.
powershell -ExecutionPolicy Bypass -File scripts/run-eval.ps1 `
  -RepoManifest docs/evaluation/public-repos.jsonl `
  -TaskFile     docs/evaluation/tasks/public-v0.1.jsonl `
  -LabelFile    docs/evaluation/labels/public-v0.1.jsonl `
  -OutDir       eval-results

# 2. Compute metrics from the captured run.
python scripts/analyze-eval.py `
  --run-dir eval-results/<run-id> `
  --labels  docs/evaluation/labels/public-v0.1.jsonl
```

Outputs land in `eval-results/<run-id>/`:

- `raw/` — full stdout/stderr per (task, mode).
- `parsed/results.jsonl` — structured mode output (see TASK_SCHEMA).
- `run-meta.json` — environment, versions, whether Mode D ran.
- `summary.csv` / `summary.json` — metrics grouped by repo, task type, and mode.

`eval-worktree/` and `eval-results/` are git-ignored. Commit only the input data
(repos/tasks/labels) and, if publishing, a curated summary — never the cloned
worktrees.

### Determinism notes

- Rankings (A/B/C) are deterministic given the same pinned commit and Agentgrep
  version. If they drift, that is a finding.
- Latency varies run to run; report p50/p95 over the run, and never compare
  latency across different machines.
- Mode D rankings depend on the embedding model version recorded in the semantic
  index meta; record the Agentgrep version in `run-meta.json`.
