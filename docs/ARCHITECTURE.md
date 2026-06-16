# Agentgrep Architecture

## Architecture summary

Agentgrep is a single local Rust CLI binary.

It runs a command, gathers cheap repository evidence, ranks or organizes that evidence, prints text or JSON, and exits.

Default architecture:

```text
CLI command
  -> repo discovery
  -> optional index load/status check
  -> rg-backed lexical recall where relevant
  -> deterministic evidence extraction
  -> ranking / graph / risk model
  -> text or JSON formatter
  -> exit
```

Agentgrep is not a daemon, server, dashboard, database, watcher, or LLM wrapper.

## Runtime model

Each command should:

1. start quickly;
2. inspect the current repository;
3. use the local index if useful and available;
4. gather only command-relevant evidence;
5. return concise text or stable JSON;
6. exit.

Default runtime must not require:

- daemon;
- file watcher;
- background indexer;
- resident LLM process;
- vector database service;
- database server;
- web dashboard.

## Current command architecture

| Command | Backend style | Index required? | JSON? | Purpose |
|---|---|---:|---:|---|
| `find <query>` | `rg` recall + ranking + optional index metadata | no | yes | Rank likely files for a query. |
| `index` | local file/symbol/edge/reference extraction | no | no | Build or inspect the lightweight index. |
| `map <path>` | index-backed file context | yes | yes | Show symbols and edges around one file. |
| `symbol <name>` | index-backed symbol lookup | yes | yes | Show definitions, uses, and nearby file context. |
| `related <query>` | index-backed graph neighborhood | yes | yes | Show nearby files, edges, symbols, and references. |
| `blast <query>` | index-backed risk heuristic | yes | yes | Estimate conservative likely impact. |

Commands that require index context should fail usefully when the index is missing, with a next action such as `agentgrep index`.

## `rg` recall floor

For `find`, `rg` is the recall floor.

```text
query
  -> build lexical patterns
  -> run rg
  -> group raw matches by file
  -> attach snippets and line ranges
  -> rank candidates
  -> optionally enrich with index signals
```

The index can improve ranking and evidence, but it should not replace raw search as the default candidate source.

This keeps `find` useful even before indexing.

## Local index

The index is optional and local.

Preferred storage path:

```text
.git/agentgrep/index.json
```

Fallback storage path:

```text
.agentgrep/index.json
```

The index currently stores:

- repo revision when available;
- file entries;
- file roles;
- content hashes;
- symbols;
- symbol references;
- file edges;
- role and symbol statistics;
- index status metadata.

The index is a lightweight cache, not a source of truth. It can be rebuilt.

## Index status

Commands should expose index state where relevant.

Common states:

| Status | Meaning |
|---|---|
| `fresh` | Index exists and matches the current repo revision. |
| `stale` | Index exists but repo revision differs. |
| `missing` | No index was found. |
| `unverifiable` | Index exists but freshness could not be proven. |
| `not_applicable` | Command did not use index context. |

`find --json` also exposes `coverage.index_used` so agents can tell whether indexed context affected the result.

## Evidence model

Agentgrep tracks why a result appears.

Evidence may include:

- `rg_match`;
- `filename_token_match`;
- `path_match`;
- `snippet_term_match`;
- `exact_phrase_match`;
- `near_phrase_match`;
- `source_role`;
- `fixture_like_match`;
- `indexed_symbol_definition`;
- `indexed_symbol_reference`;
- `indexed_edge`.

Evidence is explainability metadata. It may grow over time. Agents should not assume the set of evidence types is closed.

## Scoring and confidence

Scores are command-local ranking values.

Rules:

- scores are only meaningful within one response;
- scores are not comparable across commands;
- scores are not guaranteed stable across versions;
- exact symbol definitions should outrank broad helper/test noise;
- production evidence should outrank test/fixture evidence;
- `same_area` should be weak supporting evidence, not a dominant signal.

Confidence is a coarse label:

```text
low | medium | high
```

Confidence is not a probability. It is a user-facing summary of signal quality.

## File roles

Agentgrep classifies files into practical roles such as:

- source;
- doc;
- config;
- lockfile;
- test/fixture context;
- other.

Roles affect ranking and output framing. They are heuristics, not a formal language model.

## Symbol model

Symbols currently include lightweight Rust-oriented extraction for:

- modules;
- structs;
- enums;
- impl blocks;
- functions;
- constants.

Symbol records include:

- name;
- kind;
- file path;
- line number;
- visibility;
- signature.

The current symbol extraction is heuristic. Tree-sitter is a future index-time parser backend, not a full rewrite.

## Edge model

File edges represent local relationships.

Current edge types include:

- `declares_module`;
- `imports`;
- `references`;
- `same_area`;
- test/fixture-related relationships where available.

Edges include:

- source file;
- target file;
- edge type;
- confidence;
- reason.

`same_area` is intentionally weak. It helps show neighborhood but should not dominate ranking or risk.

## Command data flows

### `find`

```text
query
  -> rg recall
  -> grouped matches
  -> snippets and line ranges
  -> optional index metadata
  -> ranked FileCandidate list
  -> FindReport
```

`find` must work without an index.

### `index`

```text
repo root
  -> walk files
  -> classify roles
  -> extract symbols
  -> extract references
  -> extract file edges
  -> compute stats and hashes
  -> write index.json
```

The index can be cleared and rebuilt.

### `map`

```text
file path
  -> load index
  -> resolve file entry
  -> collect symbols
  -> collect incoming/outgoing edges
  -> summarize connection counts
  -> MapReport
```

### `symbol`

```text
symbol query
  -> load index
  -> exact / case-insensitive / substring lookup
  -> collect definitions
  -> collect grouped references
  -> collect nearby file edges
  -> SymbolReport
```

### `related`

```text
file or symbol query
  -> load index
  -> resolve mode
  -> score nearby files
  -> collect edges, symbols, references
  -> RelatedReport
```

### `blast`

```text
file or symbol query
  -> load index
  -> resolve mode
  -> separate production vs test/fixture evidence
  -> estimate risk level
  -> list impacted files and suggested inspection order
  -> BlastReport
```

Blast is conservative likely impact. It is not guaranteed breakage.

## JSON contract

JSON output is a first-class integration surface for agents.

The current v0.1 contract is documented in:

```text
docs/JSON_CONTRACT.md
```

Stable top-level fields are guarded by lightweight serialization-shape tests. Nested details such as scores, evidence details, and reason lists are best-effort and may evolve.

## Current completed architecture milestones

Completed:

- `rg`-backed `find`;
- coverage metadata;
- optional lightweight index;
- file map command;
- symbol lookup command;
- symbol-reference precision pass;
- related-neighborhood command;
- blast-risk command;
- index-aware `find` ranking;
- release hardening;
- JSON contract docs/tests.

## Future architecture layers

### Config file

A small `.agentgrep.toml` should eventually control:

- output limits;
- excluded paths;
- test/fixture patterns;
- ranking preferences.

### Retrieval v2

BM25 / FTS / identifier expansion / symbol graph boosts should become the default lightweight intelligence upgrade.

This layer should remain:

- local;
- deterministic;
- no daemon;
- no model;
- no resident service.

### Tree-sitter backend

Tree-sitter should be introduced as a Rust-first index-time parser backend.

Rules:

- no UX change;
- heuristics remain fallback;
- no daemon;
- no schema explosion;
- improve symbols, imports, references, and fixture/comment skipping.

### Optional hybrid semantic mode

Hybrid semantic retrieval may be useful later, but only behind an explicit flag.

Rules:

- disabled by default;
- local-only;
- no always-running model;
- no required GPU;
- no server;
- semantic candidates must be labeled;
- deterministic evidence remains primary.

### Multi-language support

Multi-language support should come after the retrieval and parser foundations are stable.

Likely order:

1. TypeScript / JavaScript;
2. Python;
3. Go;
4. Markdown/docs cross-links.

## Architecture non-goals

Agentgrep should not become:

- a language server;
- a compiler frontend;
- a repo chatbot;
- a vector DB client;
- a background indexing system;
- a dashboard app;
- a semantic search engine by default;
- an LLM reasoning agent.
