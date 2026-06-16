# Agent Guide for Agentgrep

This guide explains how coding agents should use Agentgrep.

Agentgrep is a fast local code radar. It gives agents ranked evidence, file context, symbol context, related files, and conservative impact estimates without requiring a daemon, LLM, database server, or background service.

## Remaining milestone checklist

- [x] Release hardening
- [x] JSON contract stabilization
- [ ] Dogfood on real repos
- [ ] Config file
- [ ] Retrieval v2: BM25 / FTS / identifier expansion / graph boosts
- [ ] Tree-sitter Rust backend
- [ ] Optional hybrid semantic mode behind a flag
- [ ] Multi-language support
- [ ] Packaging / integrations

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

## Future agent-facing improvements

Planned future work:

- config file for output limits and excludes;
- BM25/FTS lexical retrieval for stronger default `find`;
- Tree-sitter Rust backend for cleaner symbols/references;
- optional hybrid semantic mode behind an explicit flag;
- multi-language support;
- packaging/integrations.

Not planned for core:

- hidden LLM calls;
- always-running embeddings;
- daemon;
- watcher;
- dashboard;
- database server.
