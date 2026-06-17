# Agent Guide for Agentgrep

This guide explains how coding agents should use Agentgrep.

Agentgrep is a fast local code radar. It gives agents ranked evidence, file context, symbol context, related files, and conservative impact estimates without requiring a daemon, LLM, database server, or background service.

## Current milestone: Packaging / integrations

Completed milestones:

- [x] MVP core command loop
- [x] Release hardening
- [x] JSON contract stabilization
- [x] Retrieval v2: BM25-style lexical ranking, identifier expansion, graph boosts
- [x] Tree-sitter multi-language indexing (Rust, Python, JS, TS, Go)
- [ ] Dogfood on real repos
- [ ] Config file
- [ ] Optional hybrid semantic mode behind a flag
- [x] Packaging / integrations (current milestone)

## When to use `rg` directly vs agentgrep

Use `rg` when:

- you know the exact string and want raw match lines;
- you are piping output to another tool;
- you need one-off grep speed with no ranking;
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

When no index exists:

- `find` works with rg-backed ranking;
- `map`, `symbol`, `related`, `blast` have limited or no graph context;
- some evidence signals are absent.

Useful when: first contact with a repo, quick targeted search, simple error-message lookup.

### Indexed mode

Run `agentgrep index` once to build the local index.

When the index is present:

- `find` gains symbol-name boosts and graph context;
- `map` shows incoming/outgoing edges;
- `symbol` reports definitions and references;
- `related` uses import/reference edges;
- `blast` gives a more precise impact estimate.

The index is local, disposable, and rebuildable at any time. It is stored under `.git/` or `.agentgrep/` depending on the repo.

Check freshness:

```bash
agentgrep index --status
```

Rebuild if stale.

## Agent usage principles

### 1. Use Agentgrep before broad file reading

Prefer:

```bash
agentgrep find "query"
```

before reading large parts of the repo.

Agentgrep should help localize likely files and reduce context waste.

### 2. Use `rg`-backed search as the first radar pass

`find` works even without an index.

Good first commands:

```bash
agentgrep find "auth redirect"
agentgrep find "SearchResult"
agentgrep find "rg was not found"
```

Use `--json` if your agent can parse structured output:

```bash
agentgrep find "SearchResult" --json
```

### 3. Build the index when structural context matters

Run:

```bash
agentgrep index
```

when you need:

- file maps;
- symbol lookup;
- references;
- related files;
- blast estimates;
- improved `find` ranking/context.

Check freshness with:

```bash
agentgrep index --status
```

If the index is missing or stale, rebuild it.

### 4. Follow the normal workflow

Recommended workflow:

```text
find -> index -> map -> symbol -> related -> blast
```

Example:

```bash
agentgrep find "auth redirect"
agentgrep index
agentgrep map src/search.rs
agentgrep symbol SearchResult
agentgrep related src/search.rs
agentgrep blast src/search.rs
```

### 5. Prefer JSON for automated use

Agentgrep's JSON contract is documented in:

```text
docs/JSON_CONTRACT.md
```

Use JSON for agent planning or tool chaining:

```bash
agentgrep find "SearchResult" --json
agentgrep map src/search.rs --json
agentgrep symbol SearchResult --json
agentgrep related src/search.rs --json
agentgrep blast src/search.rs --json
```

Agents should rely on stable top-level fields, not exact score values or every nested reason string.

## Command guide

### `find <query>`

Use when:

- starting a task;
- localizing a feature;
- searching for an error message;
- looking for a symbol or concept;
- deciding which file to inspect first.

Example:

```bash
agentgrep find "missing ripgrep"
```

Use output:

- read top candidates first;
- inspect line ranges/snippets;
- use `Why` evidence to understand ranking;
- follow `Next` suggestions.

Notes:

- `find` uses `rg` as the recall floor;
- indexed context can improve ranking if available;
- scores are only relative inside one response.

### `index`

Use when:

- structural commands are needed;
- JSON should include richer graph context;
- `find` needs stronger symbol/edge signals.

Examples:

```bash
agentgrep index
agentgrep index --status
agentgrep index --clear
```

Notes:

- the index is local and disposable;
- it is stored under the git area when possible;
- it can be rebuilt at any time.

### `map <path>`

Use when:

- you already have a candidate file;
- you need its symbols;
- you need incoming/outgoing edges;
- you want quick local context before opening the file.

Example:

```bash
agentgrep map src/search.rs
```

Agent behavior:

- use symbols to pick important entrypoints;
- use incoming edges to find callers/importers;
- use outgoing edges to find dependencies;
- use `next_actions` to decide follow-up commands.

### `symbol <name>`

Use when:

- looking for definitions;
- checking where a type/function is used;
- determining whether a symbol is production or mostly test/fixture referenced.

Example:

```bash
agentgrep symbol SearchResult
```

Match modes:

- exact;
- case-insensitive exact;
- substring fallback.

Agent behavior:

- prefer exact matches;
- inspect `used_by` context;
- treat test/fixture references differently from production references.

### `related <file-or-symbol>`

Use when:

- you know the target but need neighborhood context;
- you want nearby files before editing;
- you need files connected by imports, references, symbols, or same-area edges.

Example:

```bash
agentgrep related src/search.rs
```

Agent behavior:

- inspect high-confidence related files first;
- treat `same_area` as weak evidence;
- prefer imports/references/symbol references over broad path proximity.

### `blast <file-or-symbol>`

Use before editing.

Example:

```bash
agentgrep blast src/search.rs
```

Blast answers:

```text
What might be impacted if this changes?
```

Important:

- blast is conservative likely impact;
- it is not a guarantee of breakage;
- production impact should matter more than test/fixture-only evidence;
- same-area-only impact is weak.

Agent behavior:

- inspect `risk_level` and `risk_reasons`;
- inspect high-confidence impacted files;
- follow `suggested_inspection_order`;
- do not claim that files outside the list are safe.

## JSON consumption rules

Agents should treat these as stable in v0.1:

- top-level report fields documented in `docs/JSON_CONTRACT.md`;
- `path` and `query` fields;
- `index_status` fields;
- top-level arrays such as `candidates`, `matches`, `related_files`, `impacted_files`, and `next_actions`.

Agents should treat these as best-effort:

- exact scores;
- exact reason strings;
- exact evidence ordering;
- exact line-range sets;
- exact snippet choice;
- complete reference coverage.

Do not compare scores across commands or versions.

Do not assume every possible reference is found.

## Confidence and risk interpretation

### Confidence

Confidence values:

```text
low | medium | high
```

They are coarse labels, not probabilities.

Use confidence to decide inspection order, not correctness.

### Risk level

Blast risk values should be treated as conservative estimates.

Use them to decide how much inspection is needed, not to prove safety.

## When not to use Agentgrep

Do not use Agentgrep as a replacement for:

- reading the final source file before editing;
- running tests;
- compiler or type-checker diagnostics;
- language-server completions;
- exact dependency analysis;
- security review.

Agentgrep is a radar, not proof.

## Recommended agent loop

For a code-change task:

```text
1. agentgrep find "task terms"
2. agentgrep index, if not fresh
3. agentgrep map <top-file>
4. agentgrep symbol <main-symbol>
5. agentgrep related <file-or-symbol>
6. agentgrep blast <file-or-symbol>
7. read/edit files
8. run tests/checks
```

For a bug/error task:

```text
1. agentgrep find "exact error message"
2. inspect top source result
3. agentgrep blast <file>
4. inspect production impacted files
5. edit
6. run targeted tests/checks
```

For a refactor:

```text
1. agentgrep symbol <symbol>
2. agentgrep related <symbol>
3. agentgrep blast <symbol>
4. inspect production references first
5. edit carefully
6. run wider tests/checks
```

## Claude Code / Codex prompt examples

### Claude Code (tool description style)

When configuring agentgrep as a tool in Claude Code, describe it like this:

```
agentgrep find <query>
  Searches the codebase for files likely related to the query.
  Returns ranked file candidates with line snippets and evidence.
  Use before reading large parts of the repo.
  Use --json for structured output.

agentgrep index
  Builds a lightweight local index of symbols, imports, and file edges.
  Run once before using map, symbol, related, or blast.
  Use --status to check freshness.

agentgrep map <file>
  Returns the symbol/edge context for one file.
  Shows incoming callers, outgoing dependencies, and next actions.

agentgrep symbol <name>
  Finds definitions and references for a symbol by name.
  Reports production vs test usage.

agentgrep related <file-or-symbol>
  Returns files connected by imports, references, or symbols.
  Use before editing to understand neighborhood.

agentgrep blast <file-or-symbol>
  Returns a conservative likely-impact estimate.
  Use before editing to see what else might break.
  risk_level: low | medium | high
```

### Typical Claude Code usage sequence

```bash
# Step 1: Localize likely files
agentgrep find "auth redirect" --json

# Step 2: Build index if not fresh
agentgrep index --status
agentgrep index

# Step 3: Inspect a candidate file
agentgrep map src/auth.rs --json

# Step 4: Trace a symbol
agentgrep symbol AuthState --json

# Step 5: Check neighbors before editing
agentgrep related src/auth.rs --json

# Step 6: Estimate impact before editing
agentgrep blast src/auth.rs --json
```

### Codex / OpenAI Assistants system prompt snippet

```
Available local tools:
- agentgrep find "<query>" [--json] — ranked file search over the codebase
- agentgrep index [--status] — build or check the local code index
- agentgrep map <file> [--json] — file-level symbol and edge context
- agentgrep symbol <name> [--json] — definitions and references for a name
- agentgrep related <file-or-symbol> [--json] — connected files by edges
- agentgrep blast <file-or-symbol> [--json] — conservative change impact

Use agentgrep find before opening files. Use agentgrep blast before editing.
Always use --json when you need to parse the output programmatically.
```

## Future agent-facing improvements

Planned future work:

- config file for output limits and excludes;
- optional hybrid semantic mode behind an explicit flag.

Not planned for core:

- hidden LLM calls;
- always-running embeddings;
- daemon;
- watcher;
- dashboard;
- database server.
