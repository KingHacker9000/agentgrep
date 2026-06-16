# Agentgrep Project Brief

## One-line description

Agentgrep is a fast local code radar for coding agents.

It is a disposable Rust CLI that helps agents decide where to look, what depends on what, what might break, and what to inspect next.

## Current status

Agentgrep has reached the v0.1-style core milestone.

The current command loop is complete:

```text
find -> index -> map -> symbol -> related -> blast
```

Completed project milestones:

- MVP core command loop:
  - `find`
  - `index`
  - `map`
  - `symbol`
  - `related`
  - `blast`
- Release hardening:
  - polished GitHub README;
  - CI for format/check/test;
  - local smoke script;
  - cleaner crate metadata.
- JSON contract stabilization:
  - `docs/JSON_CONTRACT.md`;
  - stable top-level JSON shape tests;
  - documented score, confidence, evidence, index, and risk semantics.

The next project phase is real-repo dogfooding, followed by configuration, retrieval improvements, parser improvements, and optional deeper retrieval experiments.

## Why this exists

Coding agents spend a large part of their work doing repository navigation:

- finding likely files for a task;
- locating symbols, routes, tests, configs, and entrypoints;
- tracing local dependencies;
- estimating what else might break;
- deciding which files to read next;
- deciding which commands or tests to run next.

Plain tools like `rg`, `find`, `tree`, and `cat` are excellent, but they return raw evidence. Agentgrep sits above those tools and turns evidence into ranked, explainable, agent-shaped output.

Agentgrep should not replace the coding agent. It should give the agent a better code radar with less context waste.

## Product thesis

The best default tool for this problem is not an always-on embedding service, a vector database, a repo chatbot, or a visual dashboard.

The best default tool is:

```text
rg recall floor + lightweight index + deterministic ranking + agent-shaped output
```

That means Agentgrep should:

- run as a normal CLI command;
- answer and exit;
- use local deterministic evidence first;
- keep `rg` as the recall floor;
- add optional indexed context for symbols, edges, references, and file roles;
- print concise text for humans;
- print stable JSON for agents;
- stay honest about confidence and uncertainty.

## Who it is for

Primary user:

- coding agents such as Codex-style agents, Claude Code-style agents, SWE agents, and local agentic coding tools.

Secondary user:

- developers who want a better command-line codebase radar than plain `rg`.

Agentgrep should be pleasant for humans, but optimized for agents.

## What Agentgrep should answer

Agentgrep should help answer:

```text
Where is this feature implemented?
What file should I read first?
Where is this symbol defined?
Who uses this symbol?
What imports or references this module?
What files are nearby in the graph?
If I change this file, what might be impacted?
What should I inspect next?
```

## Current command set

| Command | Status | Purpose |
|---|---:|---|
| `find <query>` | complete | Evidence-first search for likely files. |
| `index` | complete | Build or inspect the lightweight local repository index. |
| `map <path>` | complete | Inspect one file with indexed context. |
| `symbol <name>` | complete | Find definitions and references for a symbol. |
| `related <path-or-symbol>` | complete | Inspect nearby files, symbols, edges, and references. |
| `blast <path-or-symbol>` | complete | Estimate conservative likely impact before editing. |

The normal workflow is:

```bash
agentgrep find "auth redirect"
agentgrep index
agentgrep map src/search.rs
agentgrep symbol SearchResult
agentgrep related src/search.rs
agentgrep blast src/search.rs
```

## JSON as an agent interface

JSON is a first-class surface for Agentgrep.

Commands that support `--json` should keep stable top-level fields documented in `docs/JSON_CONTRACT.md`.

Important contract rules:

- `score` is response-local and should not be compared across commands or versions;
- `confidence` is a coarse label: `low`, `medium`, or `high`;
- `evidence` is explainability metadata and may grow over time;
- `index_status` describes whether indexed context was fresh, stale, missing, or otherwise limited;
- `blast` reports conservative likely impact, not guaranteed breakage.

## Core principles

### 1. Evidence first

Every important result should explain why it was shown.

Good:

```text
src/search.rs
Reason: defines SearchResult; references SearchCoverage; declared by main; matched query terms.
```

Bad:

```text
src/search.rs
```

### 2. `rg` remains the recall floor

Agentgrep does not replace `rg`.

For `find`, raw lexical recall should still start from `rg`. Indexing can improve ranking and add context, but it should not become the only way to discover candidates.

### 3. Index is optional

`agentgrep index` improves ranking, symbols, edges, references, and graph-aware commands.

If no index exists, `find` should still work with `rg` only.

### 4. Deterministic before model-based

Use cheap local signals first:

- `rg` matches;
- path and filename tokens;
- snippets and line ranges;
- symbol definitions;
- imports and references;
- file roles;
- test/fixture context;
- graph edges;
- risk and confidence labels.

### 5. No hidden runtime cost

Agentgrep should remain a disposable CLI.

Default usage must not require:

- daemon;
- file watcher;
- background indexing service;
- database server;
- dashboard;
- resident LLM process;
- always-running embedding service.

### 6. Low-token output

Agentgrep should return the smallest useful answer:

- top candidates;
- concise evidence;
- short snippets;
- stable line ranges;
- confidence labels;
- next actions.

It should not dump entire files or large graphs unless explicitly requested by a future option.

### 7. Honest uncertainty

Blast radius is an estimate.

Good:

```text
Risk: medium
Reason: 2 production files have direct inbound impact; test/fixture references exist.
```

Bad:

```text
Only these files are impacted.
```

## Non-goals

Agentgrep is not:

- an LLM wrapper;
- a repo chatbot;
- a semantic search SaaS;
- a vector database wrapper;
- a background indexer;
- an always-on code intelligence server;
- a dashboard-first graph explorer;
- a database server;
- a replacement for `rg`;
- an oracle that claims exact blast radius.

## Future direction

The remaining roadmap is:

1. Dogfood on real repos.
2. Config file.
3. Retrieval v2: BM25 / FTS / identifier expansion / graph boosts.
4. Tree-sitter Rust backend.
5. Optional hybrid semantic mode behind an explicit flag.
6. Multi-language support.
7. Packaging / integrations.

The strategic split is:

- BM25/FTS belongs in the default lightweight retrieval path later.
- Hybrid semantic retrieval is future opt-in only.
- LLM integration stays out of core.

## Success criteria

Agentgrep is successful if a coding agent can use it to:

- find the right first file faster than plain `rg`;
- avoid reading irrelevant files;
- understand local file/symbol context without a full repo dump;
- estimate change impact before editing;
- consume stable JSON with minimal glue code;
- run everything locally without a resident service.
