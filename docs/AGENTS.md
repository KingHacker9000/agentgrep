# AGENTS.md

## Purpose of this file

This file gives coding agents clear instructions for working on Agentgrep.

Agentgrep is a fast, disposable, evidence-first CLI for codebase navigation, search, mapping, and blast-radius estimation. It is designed for coding agents first and humans second.

Before making changes, read:

```text
PROJECT.md
ARCHITECTURE.md
AGENTS.md
```

## Core rule

Do not overbuild.

Agentgrep should start as a small Rust CLI that shells out to `rg`, ranks results with evidence, supports text and JSON output, and exits.

The first goal is not to build a full intelligence system. The first goal is to prove that agent-shaped output beats plain `rg` while preserving `rg` as the recall floor.

## What to build first

Implement only the MVP spine unless explicitly asked otherwise:

```bash
agentgrep find "query"
agentgrep find "query" --json
```

The MVP should:

- parse CLI args;
- detect repo root;
- call `rg`;
- run exact phrase and token searches where useful;
- collect and merge matches;
- group matches by file;
- score/rank files;
- attach evidence;
- expose search coverage when output is limited;
- print concise text output;
- support JSON output;
- handle missing `rg` cleanly.

## What not to build yet

Do not add any of these in the MVP:

- embeddings;
- vector database;
- daemon;
- file watcher;
- background indexer;
- local model server;
- cloud LLM calls;
- dashboard;
- graph visualization;
- persistent repo database;
- automatic code editing;
- huge architecture summary generator;
- complicated plugin system;
- full tree-sitter integration before `find` works.

After `find` quality is stable, an explicit `agentgrep index` command is allowed. It must run, write a lightweight local cache, report status, and exit. Do not add a budget flag. Do not make it required for `find`.

If a task seems to require one of these, pause and choose the smallest deterministic alternative.

## Implementation philosophy

### Prefer deterministic evidence

Every result should have reasons.

Good:

```text
src/auth/session.ts
Why: filename match, rg match on redirect, path suggests auth area.
```

Bad:

```text
src/auth/session.ts
Why: AI thinks it is relevant.
```

### Prefer small commands

Commands should be composable and easy for agents to call.

Good:

```bash
agentgrep find "auth redirect"
agentgrep map src/auth/session.ts
agentgrep blast src/auth/session.ts
```

Bad:

```bash
agentgrep analyze-everything --deep --semantic --dashboard
```

### Prefer low-token output

The CLI should not dump huge context by default.

Default output should usually include:

- top candidates;
- score or confidence;
- short reasons;
- line references;
- next suggested commands.

### Prefer JSON stability

Every command that agents may use should eventually support `--json`.

Do not break JSON schemas casually.

## Expected coding style

Use simple Rust.

Prefer:

- clear modules;
- small functions;
- explicit structs;
- readable scoring logic;
- simple errors;
- limited dependencies.

Avoid:

- clever abstractions;
- generic frameworks;
- unnecessary async;
- premature plugin systems;
- hidden global state;
- long-running background processes.

## Suggested initial Rust modules

Start with:

```text
src/main.rs      entrypoint
src/cli.rs       clap args and command definitions
src/repo.rs      repo root detection
src/search.rs    rg invocation and match parsing
src/rank.rs      grouping and scoring
src/output.rs    text/json formatting
src/types.rs     shared structs
```

Do not create future modules until they are needed.

Future modules may include:

```text
src/index/
src/symbols/
src/graph/
src/git/
src/blast/
src/tests/
src/llm/
```

## MVP output expectations

For:

```bash
agentgrep find "auth redirect"
```

Prefer output like:

```text
Top candidates:
1. src/auth/session.ts    score 0.86
   Lines: 18, 44, 91
   Why: path contains auth; matched redirect; multiple query terms found.

2. src/routes/login.ts    score 0.71
   Lines: 12, 39
   Why: route-like path; matched redirect.

Next:
- agentgrep map src/auth/session.ts
- agentgrep tests src/auth/session.ts
```

For:

```bash
agentgrep find "auth redirect" --json
```

Prefer output like:

```json
{
  "query": "auth redirect",
  "repo_root": "/repo",
  "repo_rev": "abc123",
  "latency_ms": 120,
  "candidates": [
    {
      "path": "src/auth/session.ts",
      "kind": "file",
      "score": 0.86,
      "confidence": "medium",
      "line_ranges": [{"start": 18, "end": 18}],
      "evidence": [
        {"type": "path_match", "detail": "path contains auth"},
        {"type": "rg_match", "detail": "matched redirect on line 18"}
      ]
    }
  ],
  "next_commands": [
    "agentgrep map src/auth/session.ts"
  ]
}
```

## Recall guidance

`find` should use `rg` as its recall floor.

Good behavior:

```text
rg found 42 matches across 8 files; Agentgrep shows the top 8 ranked candidates.
```

Bad behavior:

```text
Agentgrep silently drops raw rg candidates and gives no coverage signal.
```

Do not add a separate raw fallback command unless explicitly requested. Agents can call `rg` directly. Agentgrep's job is to make `find` safer by reporting coverage and limits.

## Scoring guidance

Keep scoring simple and explainable.

Useful early signals:

```text
filename contains query term
path contains query term
file contains multiple query terms
file has multiple matches
match appears in likely symbol line
path is test-related
path is config-related
path is doc-related
path is generated/vendor/build output
```

Do not introduce ML scoring in MVP.

Do not introduce embeddings in MVP.

## Dependency guidance

Acceptable early dependencies:

```text
clap
serde
serde_json
anyhow
```

Possibly later:

```text
tree-sitter
git2
ignore
grep-searcher
```

The first index should stay file-backed and simple. Do not add Neo4j, FalkorDB, Qdrant, SQLite, Tantivy, or an embedding/vector store unless explicitly requested.

Do not add LLM or vector database dependencies unless explicitly requested.

## Testing guidance

Add lightweight tests only where they protect core behavior.

Good test targets:

- scoring function;
- result grouping;
- query term extraction;
- JSON serialization;
- repo root fallback behavior.

Do not build a large test harness before the CLI works.

When reporting back, include manual commands the user can run, such as:

```bash
cargo fmt
cargo check
cargo test
cargo run -- find "auth redirect"
cargo run -- find "auth redirect" --json
```

If manual command output may be long, redirect it into files under `manual-test/` instead of printing it all in the terminal report.

Example PowerShell:

```powershell
mkdir manual-test -Force
cargo run -- find "auth redirect" > manual-test/find-auth-redirect.txt
cargo run -- find "auth redirect" --json > manual-test/find-auth-redirect.json
```

## Agent workflow for this repo

When asked to implement something:

1. Re-read the relevant docs.
2. Identify whether the request belongs to MVP or later roadmap.
3. Prefer the smallest useful implementation.
4. Avoid adding infrastructure unless necessary.
5. Keep output agent-friendly.
6. Keep behavior explainable.
7. Report changed files and reasoning clearly.

## How to handle ambiguity

If the user asks for a large feature, choose the smallest version that preserves the project direction.

Example:

User asks:

```text
Add semantic search.
```

Better response:

```text
Implemented deterministic query expansion and ranking hooks first. Did not add embeddings or a vector DB because the project constraints prefer disposable commands and evidence-first search.
```

Do not silently add heavyweight infrastructure.

## Feature priority order

Build in this order:

1. `find` text output.
2. `find --json`.
3. better ranking and evidence.
4. recall coverage metadata for `find`.
5. `index`, `index --status`, and `index --clear` with a lightweight local cache.
6. file connection edges in the index.
7. `map <path>` using indexed facts and lightweight parsing.
8. `connections <path>`.
9. tree-sitter symbols.
10. `symbol <name>`.
11. git history signals.
12. `related <path>`.
13. `blast <path|symbol>`.
14. `tests <path|symbol>`.
15. optional LLM query expansion/reranking.

## Definition of done for MVP

MVP is done when:

- the project builds as a Rust CLI;
- `agentgrep find "query"` works;
- `agentgrep find "query" --json` works;
- missing `rg` gives a clear error;
- results are grouped by file;
- results include evidence;
- output is concise;
- docs still match behavior.

## Reporting format for coding agents

When finishing a task, report:

```text
Changed files:
- path/to/file.rs — what changed

Behavior:
- what command now works

Not included:
- what was intentionally not built

Manual checks:
- cargo fmt
- cargo check
- cargo test
- cargo run -- find "example"
```

Keep reports concise.

## Final reminder

Agentgrep is not trying to become an AI coding agent.

Agentgrep gives coding agents better local evidence.

Build the radar, not the pilot.
