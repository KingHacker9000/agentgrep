# Agentgrep: Generic Agent Instructions

Agentgrep is a local, evidence-first CLI for codebase navigation. It ranks files by relevance,
provides symbol and call-graph context, and estimates change impact. No LLM, daemon, database,
or semantic search. Evidence is deterministic and local. All commands accept `--json`.

## When to use `rg` vs `agentgrep`

Use `rg` when:
- you know the exact string and want raw match lines;
- you are piping output to another tool (awk, sed, jq);
- you need regex power with no ranking overhead.

Use `agentgrep find` when:
- you want ranked file candidates for an open-ended query;
- you want structured JSON for downstream parsing;
- you want multi-term coverage (`--match all`);
- you want file-role filtering (`--role source`);
- you want `next_actions` to guide follow-up.

Rule of thumb: `rg` for exact string recall. `agentgrep` for file ranking, structural context, and agent-shaped output.

## Session start — orient before searching

Run once at the start of any session on an unfamiliar codebase:

```bash
agentgrep overview --json
```

Returns: entry points, package/crate structure, public types ranked by usage, vocabulary line.
Use vocabulary to anchor your first `find` query with codebase-native identifiers.

Lighter variants:
```bash
agentgrep overview --only vocab --json              # vocabulary only (~50 bytes)
agentgrep overview --only packages,entries --json   # structure only, no symbols
agentgrep overview --min-refs 3 --json              # filter low-signal types
agentgrep overview --full --min-refs 2 --json       # all symbols with at least 2 refs
```

## No-index vs indexed mode

Without an index (`find` only):
- Works with `rg`-backed ranking only.
- `map`, `symbol`, `related`, `blast`, `trace`, `peek`, `files`, `overview` require the index.
- Suitable for: first contact with a repo, quick single-term lookup.

With index (`agentgrep index` once per session):
- `find` gains symbol-name boosts and graph context.
- `trace` shows call graphs and dep resolution.
- `map` shows incoming/outgoing edges.
- `related` uses import/reference edges.
- `blast` gives precise impact estimates.

Check freshness before structural commands:
```bash
agentgrep index --status
```

## Command chains

### Cold start on unknown codebase

```bash
agentgrep overview --json                           # orient: vocab, packages, types
agentgrep find "<vocab-term from overview>" --brief --json  # locate files
agentgrep trace <SymbolName> --json                 # call graph for key symbol
agentgrep peek <SymbolName> --file <path> --json    # read implementation
```

### Feature localization

```bash
agentgrep find "feature name or concept" --brief --json
agentgrep map <top-result-file> --json
agentgrep related <top-result-file> --json
```

### Symbol tracing

```bash
agentgrep trace <SymbolName> --json
# index_status "found"    → peek the body
# index_status "external" → dep_package names the library; callers show usage
# index_status "not_found" → run next_actions[0] (rg fallback)
agentgrep peek <SymbolName> --file <path> --json
```

### Impact check before editing

```bash
agentgrep blast <file-or-symbol> --json
agentgrep related <file-or-symbol> --json
```

### Confirm a file path

```bash
agentgrep files "partial-name-or-glob" --json
```

## `trace` status — action triggers

| `index_status` | Meaning | Next step |
|---|---|---|
| `"found"` | Defined in this repo | Read `defined_in[]`, then `peek` the body |
| `"external"` | From a dependency | `dep_package` names the library; read `callers[]` for usage |
| `"not_found"` | Not in repo or any dep | Run `next_actions[0]` — rg fallback command |

**Empty `callers[]` does not mean unused.** Only indexed references are captured.

## Evidence and citation rules

- Always cite specific file paths and line numbers from Agentgrep output when referencing code.
- Do not claim a file is relevant without citing a ranked result or evidence signal.
- Do not claim a file is safe to change based solely on blast output — blast is a conservative estimate.
- Scores are relative within a single response; do not compare across commands or queries.
- Confidence values (`low | medium | high`) indicate inspection priority, not probability.

## What not to do

- Do not open files before running `agentgrep find`.
- Do not skip `agentgrep blast` before editing widely-connected files.
- Do not treat blast output as exhaustive — dynamic dispatch is not captured.
- Do not compare scores across queries.
- Do not treat empty `callers[]` as proof a symbol is unused.
- Do not treat `"external"` trace status as an error.
- Do not run `find` with a generic one-word query on an unfamiliar codebase — use `overview` first.

## Limitations

- Does not replace reading source files, running tests, or type-checking.
- Does not detect dynamic dispatch or runtime-constructed call paths.
- Reference coverage is high but not exhaustive — use `rg` for guaranteed completeness.
- Does not search outside the indexed repo or across git history.
