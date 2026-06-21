# Agentgrep: Codex Agent Instructions

Practical instructions for Codex-style terminal/code agents.

## Core rules

- Run `agentgrep overview` at session start on any unfamiliar repo before searching.
- Run `agentgrep find` before opening files when the target is unknown.
- Run `agentgrep blast` before editing any file with non-trivial connections.
- Use `--json` for all automated output — text format is for human display only.
- Do not run full-repo sweeps unless no narrower search is possible.

## When to use `agentgrep` vs `rg`

Use `agentgrep` when:
- the target is open-ended and you need ranked candidates;
- you need structural context (symbols, call graph, impact) before editing;
- you want `--json` for stable programmatic output.

Use `rg` when:
- you know the exact string;
- you are piping to `awk`, `sed`, or another tool;
- you need raw lines with no ranking overhead.

## Preferred command patterns

### Orient at session start

```bash
agentgrep overview --json
# Use vocabulary[] to anchor follow-up find queries.
# On large repos: agentgrep overview --min-refs 3 --json
# Types + functions: agentgrep overview --full --json
# Vocab only: agentgrep overview --only vocab --json
```

### Localize before opening

```bash
agentgrep find "<query using vocab terms>" --brief --json
# Read candidates[]. Open only top-ranked files.
# If score < 0.30 with Low-confidence note: requery using vocab line terms.
```

### Check index freshness

```bash
agentgrep index --status
agentgrep index               # rebuild if stale or missing
```

### Trace a symbol's call graph

```bash
agentgrep trace <SymbolName> --json
# index_status "found"    → peek the body with: agentgrep peek <sym> --file <path> --json
# index_status "external" → dep_package names the library; callers[] show usage context
# index_status "not_found" → run next_actions[0] (rg fallback)
```

### Read a symbol body

```bash
agentgrep peek <SymbolName> --file <path> --json
agentgrep peek <SymbolName> --context 5 --json     # with 5 lines of surrounding context
```

### Confirm a file path

```bash
agentgrep files "partial-name" --json              # substring/glob against indexed paths
```

### Inspect a candidate file

```bash
agentgrep map src/target.rs --json
# Use symbols[] to pick entry points; incoming_edges for callers; outgoing_edges for deps.
```

### Check neighbors before editing

```bash
agentgrep related src/target.rs --json
# Prefer high-confidence results (explicit imports/references over same_area proximity).
```

### Estimate impact before editing

```bash
agentgrep blast src/target.rs --json
# Check risk_level and suggested_inspection_order. Do not edit without reviewing medium/high results.
# Not exhaustive — dynamic dispatch paths are not captured.
```

## Output handling

If any command produces output too long to process inline:

```bash
agentgrep find "wide query" --json > /tmp/find-output.json
agentgrep blast src/large.rs --json > /tmp/blast-output.json
```

Reference the saved file instead of re-running.

## What not to do

- Do not open files before `agentgrep find`.
- Do not skip `agentgrep blast` before editing widely-imported files.
- Do not compare scores across different queries.
- Do not treat empty `callers[]` as proof a symbol is unused.
- Do not treat `"external"` trace status as an error — it means "from a dep".
- Do not treat blast output as exhaustive.

## System prompt snippet

Add this to your Codex system prompt to register Agentgrep:

```
Available local tools (run before opening files or making edits):

agentgrep overview [--full] [--min-refs N] [--only SECTIONS] [--json]
  Cold-start orientation. Run once per session. Returns entry points, packages, key types,
  vocabulary. Use vocabulary to anchor find queries. Sections: types,functions,packages,entries,connected,vocab.

agentgrep find "<query>" [--brief] [--role source|doc|config|test] [--json]
  Ranked file search. Use vocabulary from overview for best results. --brief gives compact
  output with a vocab: line for follow-up query anchoring.

agentgrep index [--status]
  Build or check the local code index. Required before trace, peek, files, map (full context).
  Run once per session; --status checks freshness.

agentgrep trace <symbol> [--json]
  Call graph: who calls it, where defined, what it calls.
  index_status: "found" (local) | "external" (dep; dep_package names it) | "not_found" (use rg)

agentgrep peek <symbol> [--file <path>] [--context N] [--json]
  Read a symbol's implementation body. Use after trace to read the code.

agentgrep files "<pattern>" [--json]
  List indexed files matching a substring or glob. Use to confirm exact paths.

agentgrep map <file> [--json]
  File inspection: defined symbols, incoming callers, outgoing deps.

agentgrep related <file-or-symbol> [--json]
  Connected files by imports and references. Use before editing.

agentgrep blast <file-or-symbol> [--json]
  Conservative impact estimate. risk_level: low | medium | high.
  Use before editing. Not exhaustive.

Rules:
- Run overview once at session start on unknown codebases.
- Run find before opening any file.
- Run blast before editing widely-connected files.
- Always use --json when parsing output.
- index_status "external" means "in a dependency" — not an error.
- Empty callers[] does not mean unused.
```
