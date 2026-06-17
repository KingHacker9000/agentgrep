# Changelog

All notable changes to Agentgrep are documented here.

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
