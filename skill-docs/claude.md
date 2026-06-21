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
agentgrep trace <SymbolName> --callers-body --json  # + each caller's enclosing function body
                                                    #   use when signature change → edit all callers
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

### `trace` flags — when to add them

```bash
agentgrep trace <sym> --json                    # default: definitions + callers + callees
agentgrep trace <sym> --callers-body --json     # + AST-extracted function body for each caller
                                                #   use when you're about to edit callsites —
                                                #   avoids reading each caller file manually
agentgrep trace <sym> --include-tests --json    # separates test callers into test_callers[]
                                                #   use when you need test patterns for the symbol
agentgrep trace <sym> --callers-body --include-tests --json   # both
```

`containing_function` fields per caller: `name`, `signature`, `line_start`, `line_end`, `body`, `truncated`.
`truncated: true` means body > 60 lines; header + call-site window are always included.

## `find --brief` output

```
src/path/file.rs:42:SymbolName  [score:0.82 conf:high role:source]
vocab: SymA, SymB, SymC, ...
```

**Auto-expansion**: when results are low-confidence (score < 0.30, or low confidence with score < 0.40),
`find` automatically re-queries with the best-matching vocabulary term and returns condensed results
in-line as `auto_expansion: { original_query, requery, candidates[] }`.

```
vocab: ConnectionCounts, EdgeMap, ...
auto-expansion: "postgresql database connection" → "ConnectionCounts"
  src/types.rs:281:ConnectionCounts  [0.66 high source]
  ...
note: Low confidence for "..." — auto-expanded to "ConnectionCounts"
```

If `auto_expansion` is present in the JSON response, **use its `requery` term** for follow-up
`trace` and `peek` calls. If auto-expansion also returns low scores, use `agentgrep overview --only vocab` to get the full vocabulary list.

## Output fields — stable vs best-effort

| Field | Parse? | Meaning |
|---|---|---|
| `candidates[].file_path` | ✓ stable | Use this path |
| `candidates[].score` | ✗ | Relative within one response only; do not compare across queries |
| `find.vocabulary[]` | ✓ stable | Symbol names from top results — use for follow-up queries |
| `find.auto_expansion` | ✓ stable | Present on mismatch — use `requery` for follow-up trace/peek |
| `trace.index_status` | ✓ stable | `"found"` / `"external"` / `"not_found"` |
| `trace.dep_package` | ✓ stable | Library name when status is `"external"` |
| `trace.callers[]` | ✓ stable | Production callers |
| `trace.test_callers[]` | ✓ stable | Test callers (only with `--include-tests`) |
| `trace.callers[].containing_function` | ✓ stable | AST function body (only with `--callers-body`) |
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
