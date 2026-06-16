# Agentgrep

Agentgrep is a local, evidence-first CLI for codebase navigation.

It uses `rg` as the recall floor, then ranks and explains results with lightweight index facts when available.
The index is optional. If it is missing, `find` still works with `rg` only.
There is no LLM, daemon, watcher, or background service.

## Workflow

1. `agentgrep find "query"` to localize likely files with evidence.
2. `agentgrep index` to build the lightweight repo index.
3. `agentgrep map <file>` to inspect one file in context.
4. `agentgrep symbol <name>` to find definitions and references.
5. `agentgrep related <file-or-symbol>` to inspect nearby code.
6. `agentgrep blast <file-or-symbol>` to estimate conservative impact.

## Examples

```bash
agentgrep find "auth redirect"
agentgrep index
agentgrep map src/search.rs
agentgrep symbol SearchResult
agentgrep related src/search.rs
agentgrep blast src/search.rs
```

## JSON

Use `--json` where available for stable machine-readable output:

```bash
agentgrep find "auth redirect" --json
agentgrep map src/search.rs --json
agentgrep symbol SearchResult --json
agentgrep related src/search.rs --json
agentgrep blast src/search.rs --json
```

## Notes

- `find` is search-first and evidence-backed, not semantic search.
- `index` is optional and should improve ranking and context, but never block `find`.
- `blast` is a conservative likely-impact estimate, not a guarantee of breakage.
- `find`, `map`, `symbol`, `related`, and `blast` all support `--json`.
