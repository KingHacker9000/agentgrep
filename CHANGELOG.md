# Changelog

All notable changes to Agentgrep are documented here.

## [0.2.0] — 2026-06-19

### What is in this release

**Retrieval ranking improvements (Mode B/C/D)**
Mode B `find` now uses a `raw_score` tiebreaker and a word-boundary short-term snippet discount, raising Hit@1 on the public benchmark. Mode C/D gain confidence-aware score fusion that weights BM25 and graph evidence against the per-candidate confidence score before merging semantic candidates.

**Semantic anchor guard**
Exact phrase queries and symbol-like identifiers (CamelCase, `snake_case`) now protect their deterministic top candidates from being displaced by semantic reranking. Semantic evidence annotates but cannot outrank a direct textual hit.

**Evaluation infrastructure**
- `tasks/public-v0.1.jsonl` (14 tasks) is the frozen gated baseline; `scripts/check-eval-gates.py` enforces regression thresholds.
- `tasks/public-v0.2.jsonl` (26 tasks) is a diagnostic expansion adding harder symbol-tracing, refactor-prep, impact-check, and workflow queries.
- `tasks/public-v0.3-validation.jsonl` (14 tasks) covers `sharkdp/fd` — an unseen repo for learning and tuning without gate enforcement.
- `tasks/public-holdout-v0.1.jsonl` (12 tasks) covers `sharkdp/bat` — frozen aggregate-only holdout for generalization checks.
- `scripts/run-eval.ps1`, `analyze-eval.py`, `render-eval-report.py`, and `check-eval-gates.py` form the runnable evaluation harness.

**Ranking diagnostics**
`analyze-eval.py` now emits per-mode, per-task win/miss/regression tables alongside the aggregate `summary.csv` / `summary.json`.

---

## [0.1.2] — 2026-06-17

First public release.

### What is in this release

**Local-first agent-native code search**
A disposable Rust CLI with no LLM, daemon, watcher, background service, or remote dependency. Run a command, get ranked results, exit. Works on any git repo without configuration.

**Retrieval v2: lexical, symbol, and graph ranking**
`find` now combines `rg` recall with symbol presence, file-role heuristics, and graph-edge evidence into a single ranked result. Evidence fields explain why each file appeared.

**Tree-sitter multi-language indexing**
`agentgrep index` extracts symbols from Rust, Python, JavaScript, and Go via Tree-sitter. Symbols feed into `map`, `symbol`, `related`, and `blast` commands.

**JSON contract stabilization**
All commands support `--json` with a stable documented shape. Contract is in `docs/JSON_CONTRACT.md`. Covered by top-level shape tests.

**Install and release verification**
`scripts/smoke.ps1` runs format, check, tests, and functional self-tests on the agentgrep repo itself. `scripts/verify-install.ps1` verifies an installed binary end-to-end.

**Shell completions**
`agentgrep completions <shell>` generates tab completions for Bash, Zsh, Fish, and PowerShell.

**Agent skill docs**
Ready-to-use instruction files in `skill-docs/` for Claude Code-style agents, Codex-style terminal agents, and a generic skill template usable with any coding agent.

---

*Older entries will appear here as future versions are tagged.*
