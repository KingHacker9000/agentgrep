# Agentgrep: Claude Code Agent Instructions

Practical instructions for Claude Code-style agents using Agentgrep as a codebase radar.

## Core principle

Use Agentgrep to localize before you read, and to estimate impact before you edit.
Agentgrep is a radar — it narrows the search space. You still read source and run tests to confirm.

## When to reach for Agentgrep

Before any file read for an unfamiliar target:

```bash
agentgrep find "query" --json
```

Before any edit to a non-trivial file:

```bash
agentgrep blast src/target.rs --json
```

Before any refactor touching a symbol:

```bash
agentgrep symbol SymbolName --json
agentgrep related src/target.rs --json
```

## Bug fix loop

```bash
# 1. Localize the error
agentgrep find "exact error message or key term"

# 2. Inspect the top result
agentgrep map src/likely-file.rs

# 3. Check impact before editing
agentgrep blast src/likely-file.rs

# 4. Read the relevant section of the file, edit, run tests
```

## Refactor loop

```bash
# 1. Find definition and all usages
agentgrep index
agentgrep symbol SymbolName --json

# 2. Inspect connected files
agentgrep related src/target.rs --json

# 3. Estimate blast radius
agentgrep blast src/target.rs --json

# 4. Inspect suggested files in order, edit carefully, run wider tests
```

## Feature localization loop

```bash
# 1. Search for the feature
agentgrep find "feature name or key concept"

# 2. Map the top candidate
agentgrep map src/candidate.rs

# 3. Find related files
agentgrep related src/candidate.rs

# 4. Open and read the specific files — do not open the whole repo
```

## Example: tracing AuthState

```bash
agentgrep index --status
agentgrep index

agentgrep symbol AuthState --json
# read: definitions, used_by, production vs test breakdown

agentgrep map src/auth.rs --json
# read: symbols, incoming edges (callers), outgoing edges (deps)

agentgrep related src/auth.rs --json
# read: high-confidence connected files

agentgrep blast src/auth.rs --json
# read: risk_level, suggested_inspection_order
# inspect medium/high risk files before editing
```

## Output interpretation

| Field | Meaning |
|---|---|
| `candidates[].score` | Relative ranking within this response only |
| `candidates[].why` | Evidence signals used for ranking |
| `next_actions` | Suggested follow-up commands |
| `confidence` | Inspection priority: `low / medium / high` |
| `risk_level` | Conservative blast estimate: `low / medium / high` |

Do not compare scores across queries. Do not treat confidence as a probability.

## What not to do

- Do not open large portions of the repo before running `agentgrep find`.
- Do not skip `agentgrep blast` before editing widely-connected files.
- Do not treat blast output as exhaustive — it is a conservative estimate.
- Do not assume semantic search is available — it is not.
- Do not cite evidence without a file path from Agentgrep output.

## JSON for tool use

When using Agentgrep as a tool call with structured output:

```bash
agentgrep find "auth redirect" --json
agentgrep map src/auth.rs --json
agentgrep symbol AuthState --json
agentgrep related src/auth.rs --json
agentgrep blast src/auth.rs --json
```

Stable fields (safe to parse): `query`, `path`, `candidates`, `matches`, `related_files`,
`impacted_files`, `next_actions`, `index_status`.

Best-effort fields (do not hardcode): exact scores, exact reason strings, exact evidence ordering.

See [docs/JSON_CONTRACT.md](../docs/JSON_CONTRACT.md) for the full contract.

## Future work

Evaluation of Claude Code prompt strategies across real codebase tasks is planned but not yet complete.
Claims about agent prompt effectiveness will require empirical measurement.
