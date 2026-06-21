# Agent Guide for Agentgrep

This guide explains how coding agents should use Agentgrep.

Agentgrep is a fast local code radar. It gives agents ranked evidence, file context, symbol and
call-graph context, related files, and conservative impact estimates. No daemon, LLM, database
server, or background service required. All commands accept `--json` for stable, parseable output.

## When to use `rg` directly vs agentgrep

Use `rg` when:
- you know the exact string and want raw match lines;
- you are piping output to another tool;
- you need pure grep speed with no ranking;
- you need regex power over raw text.

Use `agentgrep find` when:
- you want ranked file candidates, not raw lines;
- you want structured JSON for downstream use;
- you want symbol/graph context alongside matches;
- you want `--match all` for multi-term coverage;
- you want `--role source` to filter by file role;
- you want `next_actions` to guide follow-up.

Rule of thumb: `rg` is the recall floor. `agentgrep` sits above it and makes results agent-shaped.

## No-index mode vs indexed mode

### No-index mode

`find` works with `rg` only, even without an index.

Limitations without an index:
- `trace`, `peek`, `files`, `overview` require the index;
- `map`, `symbol`, `related`, `blast` have limited or no graph context.

Useful for: first contact with a repo, quick targeted search, simple error-message lookup.

### Indexed mode

Run `agentgrep index` once to build the local index.

With the index:
- `find` gains symbol-name boosts and graph context;
- `trace` shows call graphs and external-dep resolution;
- `map` shows incoming/outgoing edges;
- `symbol` reports definitions and references;
- `related` uses import/reference edges;
- `blast` gives a more precise impact estimate;
- `overview` provides cold-start codebase orientation.

The index is local and disposable — stored under `.git/agentgrep/` when possible. Check freshness:

```bash
agentgrep index --status
```

Rebuild if stale or missing.

## Agent usage principles

### 1. Orient before searching

On any unfamiliar codebase, run once at session start:

```bash
agentgrep overview --json
```

Returns: entry points, package/crate structure, public types ranked by reference count, vocabulary.
Use vocabulary terms to anchor your first `find` query — generic queries waste 3-5 calls that
`overview` replaces with one.

Lighter variants:
```bash
agentgrep overview --only vocab --json              # vocabulary only
agentgrep overview --only packages,entries --json   # structure, no symbols
agentgrep overview --min-refs 3 --json              # filter noise on large repos
agentgrep overview --full --min-refs 2 --json       # all symbols with signal
```

### 2. Localize before reading

Always run `find` before opening files:

```bash
agentgrep find "<vocab-informed query>" --brief --json
```

The `vocab:` line in `--brief` output lists symbol names from top candidates. If results are
weak (score < 0.30, "Low-confidence" note), use vocab terms to requery.

### 3. Build the index when structural context matters

Run `agentgrep index` when you need trace, map, related, blast, overview, or peek.
Check freshness first with `--status`.

### 4. Follow the canonical workflow

```
Session start  : overview → find --brief
Navigation     : trace → peek → files
Before editing : related → blast
```

### 5. Prefer JSON for automated use

All commands accept `--json`. Use it in any automated or agent context. Stable fields are safe
to parse; best-effort fields may vary. See [docs/JSON_CONTRACT.md](JSON_CONTRACT.md).

## Command guide

### `overview`

Run once per session before your first `find` call.

```bash
agentgrep overview --json
agentgrep overview --only vocab --json
agentgrep overview --full --min-refs 2 --json
agentgrep overview --only packages,entries --json
```

Output sections:
- `entry_points` — main files (main.rs, __init__.py, index.ts, etc.)
- `packages` — top-level source directories grouped by prefix (workspace-aware)
- `key_types` — public structs/enums/traits ranked by reference count (default: top 20)
- `key_functions` — public functions, shown only with `--full`
- `most_connected` — file pairs sharing the most edges
- `vocabulary` — top symbol names for query anchoring

Flags:
- `--full` — all public types + all public functions, uncapped
- `--min-refs N` — exclude symbols with fewer than N references
- `--only SECTIONS` — comma-separated subset: `types,functions,packages,entries,connected,vocab`

### `find <query>`

Use at the start of any task to locate relevant files.

```bash
agentgrep find "auth redirect" --brief --json
agentgrep find "SyntaxMapping" --role source --json
agentgrep find "error handling" --match all --exclude-docs --json
```

Output:
- `candidates[]` — ranked files with evidence snippets and symbol matches
- `vocabulary[]` — top symbol names from candidates (use for follow-up queries)
- `next_actions[]` — suggested follow-up commands
- `note` — mismatch description when results are low-confidence
- `auto_expansion` — present on mismatch: automatic re-query result (see below)

`--brief` format:
```
src/path/file.rs:42:SymbolName  [score:0.82 conf:high role:source]
vocab: SymA, SymB, SymC, ...
auto-expansion: "original query" → "VocabTerm"
  src/path/file.rs:42:VocabTerm  [0.66 high source]
  ...
note: Low confidence for "..." — auto-expanded to "VocabTerm"
```

**Auto-expansion**: when results are low-confidence (no strong evidence, score < 0.30 or top
confidence `low` with score < 0.40), `find` automatically re-queries with the best-matching
vocabulary term and returns condensed top-5 results in `auto_expansion`.

If `auto_expansion` is present in the JSON response: use `auto_expansion.requery` for all
follow-up `trace` and `peek` calls. Do not re-query manually with the same original query.

### `trace <symbol>`

Call graph for a symbol. The `index_status` field is an action trigger:

| `index_status` | Meaning | What to do |
|---|---|---|
| `"found"` | Defined in this repo | Read `defined_in[]`, then `agentgrep peek <sym> --file <path>` |
| `"external"` | From a dependency | `dep_package` names the library if resolved; check `callers[]` for usage context |
| `"not_found"` | Not in repo or any dep | Run `next_actions[0]` — always an `rg` fallback |

```bash
agentgrep trace SyntaxMapping --json
agentgrep trace Blueprint --json

# When you need to edit every callsite — avoids reading each caller file manually:
agentgrep trace SyntaxMapping --callers-body --json

# To study test patterns for the symbol:
agentgrep trace SyntaxMapping --include-tests --json

# Both:
agentgrep trace SyntaxMapping --callers-body --include-tests --json
```

**Empty `callers[]` does not mean unused** — only indexed references are captured.
**`"external"` is not an error** — the symbol is defined in a dependency, not this repo.

Flags:
- `--callers-body` — adds `containing_function` to each `callers[]` entry: the AST-extracted
  enclosing function body (≤ 60 lines; truncated with call site always visible). Capped at 10 callers.
  Fields: `name`, `signature`, `line_start`, `line_end`, `body`, `truncated`.
- `--include-tests` — routes test-file callers to `test_callers[]` instead of mixing into `callers[]`.
  With `--callers-body`, test caller bodies are also extracted (max 5).

### `peek <symbol>`

Read a symbol's implementation body without opening the file.

```bash
agentgrep peek SyntaxMapping --json
agentgrep peek SyntaxMapping --file src/syntax_mapping.rs --json
agentgrep peek add_url_rule --context 5 --json
```

Use after `trace` returns `defined_in[]`. Pass `--file` when the symbol is defined in multiple
files. `--context N` adds N surrounding lines for call-site context.

### `files <pattern>`

List indexed files matching a path pattern.

```bash
agentgrep files "auth" --json
agentgrep files "src/*.rs" --json
```

Use to confirm exact file paths before opening, or to check whether a file is indexed.
Supports substring and glob matching against full relative paths.

### `index`

Build or refresh the local repository index.

```bash
agentgrep index
agentgrep index --status
agentgrep index --clear
```

Required before structural commands. Run once per session. The index is local and disposable.

### `map <path>`

Full file inspection: role, defined symbols, incoming callers, outgoing dependencies.

```bash
agentgrep map src/search.rs --json
```

- `symbols[]` — every symbol defined in the file
- `incoming_edges[]` — files that import or call into this file
- `outgoing_edges[]` — files this file imports or references
- `next_actions[]` — follow-up commands

### `symbol <name>`

Definitions and reference sites for a symbol name. Tries exact → case-insensitive → substring match.

```bash
agentgrep symbol SearchResult --json
```

Check `used_by` context to distinguish production references from test/fixture-only references.
Prefer `trace` when you need the full call graph.

### `related <file-or-symbol>`

Files connected by imports, symbol references, or shared edges.

```bash
agentgrep related src/search.rs --json
```

- High-confidence results share explicit import/reference edges.
- `same_area` results share only directory proximity — treat as weak evidence.

### `blast <file-or-symbol>`

Conservative impact estimate: what else might break if this changes.

```bash
agentgrep blast src/search.rs --json
```

- `risk_level` (`low | medium | high`) — guides inspection depth
- `suggested_inspection_order` — files to check before editing
- Not exhaustive: dynamic dispatch and runtime paths are not captured.
- Do not claim files outside the list are safe to change.

## JSON consumption rules

### Stable fields (safe to parse)

- `candidates[].file_path`, `candidates[].line_range`
- `find.vocabulary[]`, `find.auto_expansion.requery`, `find.auto_expansion.candidates[]`
- `trace.index_status`, `trace.dep_package`, `trace.defined_in[]`, `trace.callers[]`
- `trace.test_callers[]` (with `--include-tests`), `trace.callers[].containing_function` (with `--callers-body`)
- `overview.vocabulary[]`, `overview.key_types[]`, `overview.entry_points[]`
- `next_actions[]`
- All top-level `path`, `query`, `index_status` fields

### Best-effort fields (do not hardcode)

- Exact scores
- Exact reason strings and evidence ordering
- Exact snippet choice and line ranges
- Complete reference coverage (use `rg` for guaranteed completeness)

Do not compare scores across commands, queries, or versions.
Do not assume every possible reference is found.

## Confidence and risk interpretation

**Confidence** (`low | medium | high`): coarse inspection priority, not a probability.
Use it to order which files to look at first, not to decide whether results are correct.

**Risk level** (blast): conservative estimate, not proof of impact.
Use it to decide how much inspection is needed before editing.

## When not to use Agentgrep

Agentgrep is a radar, not a proof system. It does not replace:
- reading the final source file before editing;
- running tests or type-checking;
- compiler or language-server diagnostics;
- security review;
- exact dependency analysis (use `cargo tree`, `pip show`, etc.);
- searching across multiple repos or git history.

## Recommended agent loop

### Code-change task

```
1. agentgrep overview              — orient: vocab, packages, entry points
2. agentgrep find "<task>" --brief — locate relevant files
3. agentgrep index (if not fresh)  — ensure structural commands work
4. agentgrep trace <main-symbol>   — call graph; act on index_status
5. agentgrep peek <symbol>         — read implementation
6. agentgrep related <file>        — understand neighborhood
7. agentgrep blast <file>          — estimate impact
8. read / edit files
9. run tests and checks
```

### Bug / error task

```
1. agentgrep find "exact error message" --brief
2. agentgrep trace <symbol-near-error>       — confirm local or external
3. agentgrep peek <symbol> --context 5       — read context around the bug
4. agentgrep blast <file>                    — impact before fixing
5. edit
6. run targeted tests
```

### Refactor task

```
1. agentgrep trace <symbol>                  — all callers and callees
2. agentgrep related <file>                  — connected files
3. agentgrep blast <symbol>                  — impact radius
4. inspect production references first
5. edit carefully
6. run wider tests
```

## Prompt examples for integration

### Claude Code — tool description style

```
agentgrep overview [--full] [--min-refs N] [--only SECTIONS] [--json]
  Cold-start orientation. Run once per session. Returns entry points, packages, key types,
  vocabulary. Use vocabulary to anchor find queries with codebase-native identifiers.
  --full: all types + functions uncapped. --min-refs N: filter low-signal symbols.
  --only: comma-separated sections (types,functions,packages,entries,connected,vocab).

agentgrep find "<query>" [--brief] [--role source|doc|config|test] [--json]
  Ranked file search. Use vocabulary from overview for best results. --brief gives compact
  output ending with a vocab: line for follow-up anchoring.
  On mismatch, auto_expansion in response contains best-match requery — use it for follow-up.

agentgrep index [--status]
  Build or check the local code index. Required before trace, peek, files, overview.
  Run once per session; --status checks freshness.

agentgrep trace <symbol> [--callers-body] [--include-tests] [--json]
  Call graph. index_status: "found" (local), "external" (dep; dep_package names it),
  "not_found" (run next_actions[0]). Empty callers[] ≠ unused.
  --callers-body: adds containing_function body to each caller (use before editing callsites).
  --include-tests: routes test callers to test_callers[].

agentgrep peek <symbol> [--file <path>] [--context N] [--json]
  Read a symbol's implementation. Use after trace identifies defined_in[]. --context N
  adds N surrounding lines.

agentgrep files "<pattern>" [--json]
  Indexed files matching substring or glob. Use to confirm paths.

agentgrep map <file> [--json]
  File inspection: symbols, incoming callers, outgoing deps.

agentgrep related <file-or-symbol> [--json]
  Connected files by imports and references. Use before editing.

agentgrep blast <file-or-symbol> [--json]
  Conservative impact estimate. risk_level: low | medium | high.
```

### Codex / OpenAI Assistants system prompt snippet

```
Available local tools:
- agentgrep overview [--only SECTIONS] [--min-refs N] [--full] [--json]
    Cold-start orientation (run once per session). Sections: types,functions,packages,entries,connected,vocab
- agentgrep find "<query>" [--brief] [--json]
    Ranked file search. Use vocab from overview to anchor queries.
    On mismatch, auto_expansion.requery in response has the best vocab match — use it for follow-up.
- agentgrep index [--status]
    Build/check the local code index. Required before structural commands.
- agentgrep trace <symbol> [--callers-body] [--include-tests] [--json]
    Call graph. index_status "found"=local, "external"=dep (dep_package names it), "not_found"=use rg.
    --callers-body: adds enclosing function body per caller. --include-tests: splits test callers.
- agentgrep peek <symbol> [--file <path>] [--context N] [--json]
    Read a symbol's body. Use after trace.
- agentgrep files "<pattern>" [--json]
    Confirm indexed file paths.
- agentgrep map <file> [--json]
    File symbols and edges.
- agentgrep related <file-or-symbol> [--json]
    Connected files by imports/references.
- agentgrep blast <file-or-symbol> [--json]
    Conservative impact estimate before editing.

Rules: run overview once at session start. Run find before opening files.
Run blast before editing widely-connected files. Always use --json for parsing.
"external" trace status = in a dependency, not an error. Empty callers[] ≠ unused.
If find returns auto_expansion, use auto_expansion.requery for follow-up calls.
Add --callers-body to trace when you need to edit every callsite.
```
