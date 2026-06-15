# Agentgrep Architecture

## Architecture summary

Agentgrep is a single local CLI binary that runs a command, gathers cheap repository evidence, ranks the result, prints concise output, and exits.

The default architecture is:

```text
Rust CLI
  -> repo discovery
  -> query parsing
  -> rg-backed recall search
  -> optional lightweight index facts
  -> scoring/ranking
  -> text or JSON formatter
  -> exit
```

The long-term architecture is:

```text
rg recall floor + lightweight index + tree-sitter + file connection graph + git history + optional test map + optional LLM reranker
```

The core product is not raw search. The core product is evidence-backed ranking and agent-shaped output.

## Runtime model

Agentgrep should be disposable.

Each command should:

1. start quickly;
2. inspect the current repository or supplied path;
3. gather only the evidence needed for that command;
4. print a concise result;
5. exit.

MVP must not require:

- daemon;
- file watcher;
- background indexer;
- vector database;
- resident LLM process;
- web dashboard.

A later explicit `agentgrep index` command is allowed. It must run, write a small local cache, report what it found, and exit. It must not become a daemon, watcher, or required service.

## Initial implementation strategy

Start from scratch as a Rust CLI, but do not reimplement ripgrep at first.

Use `rg` as the search backend.

```text
agentgrep = orchestrator and ranking layer
rg        = recall floor and lexical search backend
index     = optional repository-facts cache
git       = history backend
tree-sitter later = symbol/AST backend
```

Why not fork `rg`:

- `rg` already solves raw fast recursive search extremely well;
- Agentgrep's value is not being a faster grep;
- forking `rg` would force the project to live inside a search-engine codebase;
- the product needs workflow commands, scoring, evidence, JSON, maps, and blast-radius estimates.

If shelling out to `rg` becomes limiting later, Agentgrep can replace specific pieces with Rust crates from the same ecosystem, such as ignore/walkdir, grep-searcher, regex-automata, or similar libraries.

## MVP command set

The first version should implement:

```bash
agentgrep find "query"
agentgrep find "query" --json
```

Do not implement `map`, `symbol`, `blast`, `tests`, embeddings, LLM calls, or a dashboard in the first pass unless explicitly requested.

The next architectural step after `find` quality is not a raw `rg` fallback command. Agents can call `rg` directly. The next step is a recall contract inside `find` and then an explicit lightweight `index` command.

## Planned command set

Commands should remain small and composable.

| Command | Purpose |
|---|---|
| `find <query>` | Rank likely files/symbols for a task or concept while preserving `rg` recall evidence. |
| `index` | Build a fast local repository-facts cache. |
| `index --status` | Show whether the index exists, what it contains, and whether it matches the current repo revision. |
| `index --clear` | Remove the local Agentgrep index cache. |
| `map <path|symbol>` | Show local structure around a file or symbol. |
| `connections <path>` | Show direct file-level incoming/outgoing connections. |
| `symbol <name>` | Find definitions and references. |
| `trace <path|symbol>` | Follow dependency edges. |
| `blast <path|symbol|description>` | Estimate impacted files/tests and risk. |
| `tests <path|symbol>` | Suggest tests to run first. |
| `related <path|symbol>` | Show git co-change and neighborhood hints. |
| `plan <task>` | Suggest next Agentgrep commands for a coding task. |

## Data flow for `find`

```text
User/agent query
  -> parse query
  -> preserve exact phrase
  -> derive seed terms
  -> run rg exact-phrase pass when useful
  -> run rg token/term pass
  -> collect and merge matches
  -> group by file
  -> add optional index facts when available
  -> score files
  -> attach evidence and coverage metadata
  -> print top candidates
```

Example:

```bash
agentgrep find "auth redirect"
```

Possible internal steps:

```text
query terms: auth, redirect
rg search: auth|redirect
candidate files: grouped by path
extra signals: filename match, test path, config path, docs path
score: weighted heuristic
output: top files with reasons
```

## Recall contract for `find`

`find` is not a replacement for raw `rg`; it is ranked `rg` plus structure.

The contract is:

```text
rg remains the recall floor.
Agentgrep may reorder, group, and explain results.
Agentgrep must not silently hide that rg found more candidates than it displayed.
```

Text and JSON output should eventually expose:

```text
raw_rg_match_count
raw_rg_candidate_file_count
shown_candidate_count
result_limit
limited true/false
index_used true/false
index_status missing|fresh|stale|partial
```

If output is limited, `find` should say so clearly. Agents can then refine the query or call raw `rg` themselves.

## Lightweight index

After `find` is reliable, Agentgrep should support:

```bash
agentgrep index
agentgrep index --status
agentgrep index --clear
```

There should be no `--budget` flag. The command should be fast by default by doing only cheap deterministic work. On very large repositories it may gracefully produce a partial index and report what was skipped.

The index should be stored locally, preferably outside tracked source files, for example:

```text
.git/agentgrep/index.json
```

If `.git` is unavailable, use a local cache directory and report its path.

The index should contain repository facts, not AI summaries:

```text
files
file roles
symbol definitions
imports and exports
file-level connections
likely tests
package/build hints
git revision and content hashes
```

Missing or stale index data must never block `find`; the command falls back to rg-only ranking.

## File connection graph

Agentgrep should build a lightweight file-level graph before attempting deep semantic analysis.

Useful edge types:

```text
contains_symbol
imports_file
imports_symbol
exports_symbol
calls_symbol
references_symbol
tested_by
configured_by
co_changed_with
```

Each edge should carry:

```text
source file
target file or symbol
relation
evidence line if available
confidence: extracted|inferred|ambiguous
```

Prefer `extracted` edges from deterministic syntax/import analysis. Use `inferred` only for weaker evidence such as name matching or git co-change.

## Evidence model

Agentgrep should track why each candidate was selected.

Evidence types may include:

```text
rg_match
path_match
filename_match
symbol_match
import_edge
reference_edge
test_proximity
git_cochange
churn
config_match
doc_match
```

MVP only needs a few:

```text
rg_match
exact_phrase_match
near_phrase_match
path_match
filename_match
test_proximity
config_match
doc_match
coverage_summary
```

Indexed stages may add:

```text
symbol_match
file_connection
import_edge
export_edge
reference_edge
call_edge
tested_by
```

## Scoring model v0

The first scoring model should be simple, explainable, and monotonic.

Example scoring signals:

```text
+ exact query term in filename
+ exact query term in path
+ multiple query terms in same file
+ match appears in symbol-looking line
+ match appears in route/config/test file
+ file is not ignored by gitignore
- generated/minified/vendor file
- lockfile unless query directly targets dependency data
```

The score should not pretend to be semantic truth. It is just a ranking heuristic.

Every score should be explainable through evidence.

## Output model

Default output should be concise text.

Example:

```text
agentgrep find "auth redirect"

Top candidates:
1. src/auth/session.ts    score 0.86
   Lines: 18, 44, 91
   Why: filename match: auth; matched redirect; imported by login route.

2. src/routes/login.ts    score 0.73
   Lines: 12, 39
   Why: path suggests route; matched redirect; imports auth session.

Next:
- agentgrep map src/auth/session.ts
- agentgrep tests src/auth/session.ts
```

JSON output should be stable and machine-readable.

Example schema:

```json
{
  "query": "auth redirect",
  "repo_root": "/path/to/repo",
  "repo_rev": "abc123",
  "latency_ms": 123,
  "candidates": [
    {
      "path": "src/auth/session.ts",
      "kind": "file",
      "score": 0.86,
      "confidence": "medium",
      "line_ranges": [
        {"start": 18, "end": 18},
        {"start": 44, "end": 44}
      ],
      "evidence": [
        {"type": "filename_match", "detail": "path contains 'auth'"},
        {"type": "rg_match", "detail": "matched 'redirect' on line 44"}
      ]
    }
  ],
  "next_commands": [
    "agentgrep map src/auth/session.ts",
    "agentgrep tests src/auth/session.ts"
  ]
}
```

## Repository discovery

Agentgrep should detect the repository root using:

1. `git rev-parse --show-toplevel` if available;
2. current working directory fallback.

It should respect ignored files by default through `rg` behavior.

Add explicit flags later if needed:

```bash
agentgrep find "query" --root path/to/repo
agentgrep find "query" --no-ignore
```

## Search backend

MVP should shell out to `rg`.

Suggested behavior:

- require `rg` if available;
- produce a clear error if `rg` is missing;
- later optionally provide a slower fallback;
- capture line number, path, and snippet;
- avoid huge output by limiting matches per file and total files.

Possible `rg` options to evaluate:

```bash
rg --line-number --column --smart-case --hidden --glob '!{.git,target,node_modules,dist,build}' <query>
```

Be careful with `--hidden`. Default behavior should probably respect common ignores and avoid hidden/vendor/build folders unless explicitly requested.

## Structural layer later

After MVP `find` and the recall contract, add the lightweight `index` command.

Then add tree-sitter parsing.

Tree-sitter should provide:

- function names;
- class names;
- method names;
- import statements;
- export statements;
- route-ish declarations where language queries support them;
- symbol line ranges.

The first structural feature should probably be `map <path>`, not global repo indexing.

`map <path>` can parse one file on demand and return local structure quickly.

## Import/reference graph later

The first graph should be cheap, local, and file-centered.

Start with direct file connections:

```text
file A imports file B
file C imports file A
file T tests file A
file A defines symbol X
file B calls or references symbol X
```

Then add symbol-level and reference-level edges if exact language tooling or simple search makes them reliable.

Do not build a full graph database for MVP. The index should be a compact file on disk, not Neo4j/FalkorDB/Qdrant.

Store only facts that improve `find`, `map`, `connections`, `related`, `blast`, or `tests`.

## Git history signals later

Git can provide useful risk and relationship signals without a watcher.

Potential signals:

- recently changed files;
- files that often co-change;
- churn/hotspot files;
- files changed in current branch;
- likely ownership areas.

Example later command:

```bash
agentgrep related app/meeting_session.py
```

Potential output:

```text
Historically related:
- app/routers/meeting_sessions.py — co-changed 8 times
- app/schemas.py — co-changed 4 times
- tests/test_meeting_sessions.py — co-changed 3 times
```

## Blast-radius design later

Blast radius should be an estimate with confidence.

Potential inputs:

- direct references;
- imports and reverse imports;
- public API or signature change hints;
- likely tests;
- git co-change;
- churn;
- build target fan-out if available.

Potential output:

```text
Risk: medium
Confidence: medium

Likely impacted:
1. app/routers/meeting_sessions.py
   Evidence: imports target file; calls MeetingSession.start.
2. tests/test_meeting_sessions.py
   Evidence: test proximity and symbol match.

Blind spots:
- dynamic imports not analyzed
- no coverage map found
```

Never claim exact impact.

## Optional LLM integration later

LLM use must be optional and bounded.

Possible flags:

```bash
agentgrep find "query" --llm
agentgrep find "query" --rerank
agentgrep blast app/session.py --summarize
```

Allowed LLM tasks:

- expand vague natural language into search terms;
- rerank top 10 to 30 deterministic candidates;
- summarize evidence;
- suggest next commands.

Disallowed default LLM tasks:

- repo-wide semantic indexing;
- hidden background inference;
- whole-repo architecture claims;
- invented blast radius;
- sending repo contents to cloud without explicit opt-in.

## Performance goals

Normal commands should target:

```text
small repo: p50 under 500 ms
medium repo: p50 under 2 seconds
large repo: graceful degradation with limits
```

These are goals, not guarantees.

The CLI should expose latency in JSON output.

## Error handling

Errors should be direct and actionable.

Examples:

```text
Error: rg was not found. Install ripgrep or run with --backend internal once supported.
```

```text
Error: no readable files found under /path/to/repo.
```

```text
Warning: not inside a git repository; git history signals disabled.
```

Warnings should not block commands unless the missing dependency is required.

## Suggested Rust project structure

Initial structure:

```text
agentgrep/
  Cargo.toml
  src/
    main.rs
    cli.rs
    repo.rs
    search.rs
    rank.rs
    output.rs
    types.rs
  PROJECT.md
  ARCHITECTURE.md
  AGENTS.md
```

Later structure:

```text
src/
  index/
  symbols/
  graph/
  git/
  blast/
  tests/
  llm/
```

Do not create the later modules until needed.

## Dependency preferences

Reasonable early Rust crates:

```text
clap        CLI parsing
serde       JSON serialization
serde_json  JSON output
anyhow      error handling
thiserror   typed errors if needed
```

Avoid heavy dependencies unless clearly justified.

Do not add tree-sitter dependencies until implementing `map` or `symbol`.

Do not add LLM dependencies in MVP.

## Testing strategy

Prefer small tests around pure logic:

- query parsing;
- result grouping;
- scoring;
- JSON serialization.

Avoid creating a large fake test framework before the CLI spine works.

Keep tests lightweight and fast.

## Design constraint summary

Agentgrep should be:

```text
local
fast
disposable
CLI-native
evidence-backed
agent-shaped
structure-aware
LLM-optional
```

Agentgrep should not be:

```text
always-on
embedding-first
dashboard-first
daemon-first
cloud-dependent
graph-database-first
LLM-as-oracle
```
