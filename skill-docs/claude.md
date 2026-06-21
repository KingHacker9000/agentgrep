# Agentgrep: Claude Code Agent Instructions

## Core principle

**Localize before you read. Estimate impact before you edit.**

Agentgrep is a radar — narrow the search space first, then read source and run tests to confirm.
All commands accept `--json` for stable, parseable output. Always use `--json` in tool-call contexts.

## Session start: orient before searching

Run once at the start of any session on an unfamiliar codebase:

```bash
agentgrep overview --json
```

Returns entry points, package/crate structure, public types ranked by reference count, and a
vocabulary line. **Use vocabulary terms to anchor your first `find` query** — codebase-native
identifiers outperform generic terms.

Lighter variants:
```bash
agentgrep overview --only vocab --json              # vocabulary line only (~50 bytes)
agentgrep overview --only packages,entries --json   # structure only, no symbols
agentgrep overview --min-refs 3 --json              # filter low-signal types
agentgrep overview --full --min-refs 2 --json       # all types + functions with signal
```

Do not skip `overview` on an unfamiliar repo — generic queries waste 3-5 calls finding
vocabulary that `overview` gives in one.

## Task workflows

### Feature localization

```bash
agentgrep overview --only vocab --json              # 1. get vocabulary
agentgrep find "<vocab-informed query>" --brief --json  # 2. locate files
agentgrep trace <SymbolName> --json                 # 3. call graph for key symbol
agentgrep peek <SymbolName> --file <path> --json    # 4. read implementation
```

### Bug fix

```bash
agentgrep find "exact error text or key term" --brief --json
agentgrep trace <symbol-near-error> --json
agentgrep peek <symbol> --file <file> --context 5 --json
agentgrep blast <file> --json                       # check impact before editing
```

### Refactor

```bash
agentgrep trace <SymbolName> --json                 # all callers and callees
agentgrep related <file> --json                     # connected neighbors
agentgrep blast <symbol> --json                     # conservative impact radius
```

### Confirm a file path

```bash
agentgrep files "partial-name-or-glob" --json       # substring / glob against indexed paths
```

## `trace` status — action triggers

`index_status` tells you exactly what to do next:

| `index_status` | Meaning | Next step |
|---|---|---|
| `"found"` | Defined in this repo | Check `defined_in[]`, then `agentgrep peek <sym> --file <path>` |
| `"external"` | From a dependency | `dep_package` names the library if resolved; read `callers[]` for usage context |
| `"not_found"` | Not in repo or any dep record | Run `next_actions[0]` — always an `rg` fallback command |

**Empty `callers[]` does not mean unused** — only indexed references are captured.
**`"external"` is not an error** — it means the symbol is in a library, not this repo.

## `find --brief` output

```
src/path/file.rs:42:SymbolName  [score:0.82 conf:high role:source]
vocab: SymA, SymB, SymC, ...
```

If the top score is below 0.30 and a "Low-confidence" note appears, the query terms don't match
codebase vocabulary. Use the `vocab:` line terms to requery — they come from actual symbol names
in the top candidates.

## Output fields — stable vs best-effort

| Field | Parse? | Meaning |
|---|---|---|
| `candidates[].file_path` | ✓ stable | Use this path |
| `candidates[].score` | ✗ | Relative within one response only; do not compare across queries |
| `trace.index_status` | ✓ stable | `"found"` / `"external"` / `"not_found"` |
| `trace.dep_package` | ✓ stable | Library name when status is `"external"` |
| `trace.callers[]` | ✓ stable | Files that call this symbol |
| `trace.defined_in[]` | ✓ stable | Definition locations |
| `overview.vocabulary[]` | ✓ stable | Key symbol names for query anchoring |
| `next_actions[]` | ✓ stable | Follow these |
| `confidence` | ✗ | Inspection priority, not probability |
| `risk_level` | ✗ | Conservative estimate, not proof |

## What not to do

- **Do not read files before `agentgrep find`.** One `find --brief` call costs ~100 bytes; opening a wrong file costs thousands.
- **Do not skip `agentgrep blast`** before editing a file that map or related shows as widely connected.
- **Do not compare scores across queries.** Scores are relative within one response only.
- **Do not treat empty `callers[]` as proof a symbol is unused.** The index captures most but not all references.
- **Do not treat `"external"` status as an error.** It means the symbol is in a dependency.
- **Do not run `find` with a generic one-word query** without first getting vocabulary from `overview`.
- **Do not assume blast is exhaustive.** Dynamic dispatch and runtime paths are not captured.

## Limitations

Agentgrep is a radar, not a proof system. It does not:
- replace reading the final source file before editing;
- run tests or type-check;
- guarantee complete reference coverage (use `rg` for that);
- detect dynamic dispatch or runtime-constructed call paths;
- search outside the indexed repo.
