# Agentgrep Project Brief

## One-line description

Agentgrep is a fast, disposable, evidence-first CLI that helps coding agents understand a codebase: where to look, what depends on what, what might break, and what to run next.

## Why this exists

Coding agents spend a large part of their workflow doing the same basic navigation over and over:

- finding the likely files for a task;
- locating symbols, routes, tests, configs, and entrypoints;
- tracing local dependencies;
- estimating what else might break;
- deciding what files to read next;
- deciding what tests or checks to run.

Today, agents usually do this with plain shell tools such as `ls`, `find`, `tree`, `rg`, `cat`, and sometimes language-specific commands. These tools are powerful, but they return raw evidence rather than agent-shaped answers.

Agentgrep exists to sit between raw search and full autonomous reasoning. It should not replace the coding agent. It should give the agent better evidence, ranked results, and small tactical maps so the agent can reason faster with less context.

## Product thesis

The best tool for this problem is not an always-on embedding service, not a vector database, and not a visual graph dashboard.

The best tool is:

```text
search-first + structure-aware + agent-shaped + disposable
```

That means:

- run as a normal CLI command;
- answer quickly and exit;
- use deterministic local evidence first;
- treat `rg` as the recall floor for `find`;
- shell out to `rg` initially instead of reimplementing search;
- expose whether `find` searched fully or truncated results;
- optionally build a fast local index with cheap repository facts;
- add lightweight structural signals such as symbols, imports, references, file connections, tests, and git history;
- produce concise output with reasons;
- support JSON output for coding agents;
- keep any LLM use optional, bounded, and late-stage.

## Who it is for

Primary user:

- coding agents such as Codex-style agents, Claude Code-style agents, SWE agents, and local agentic coding tools.

Secondary user:

- developers who want a better command-line codebase radar than plain `rg`.

Agentgrep should be pleasant for humans, but optimized for agents.

## What Agentgrep should do

Agentgrep should help answer questions like:

```text
Where is this feature implemented?
What file should I read first?
Where is this symbol defined?
Who calls this function?
What imports this module?
What tests are likely relevant?
If I change this file, what might break?
What are the key files in this subsystem?
What command should the agent run next?
```

## What Agentgrep is not

Agentgrep is not:

- a semantic search SaaS;
- a background indexer;
- a vector database wrapper;
- a repo chatbot;
- a dashboard-first graph explorer;
- an always-on code intelligence server;
- a replacement for the coding agent;
- an oracle that claims exact blast radius.

## Core principles

### 1. Evidence first

Every important result should explain why it was shown.

Bad:

```text
app/session.py
```

Good:

```text
app/session.py
Reason: filename match, defines SessionManager, imported by app/router.py, referenced by tests/test_session.py.
```

### 2. Deterministic first

Use cheap deterministic signals before using model-based reasoning.

Good first-class signals:

- `rg` matches;
- exact phrase matches;
- path and filename matches;
- symbol definitions;
- file-level connections;
- imports and exports;
- references and calls;
- nearby tests;
- git co-change history;
- churn and risk hotspots;
- build or package boundaries when available.

### 3. LLM optional and late

The coding agent already has an LLM. Agentgrep should not become a second hidden reasoning agent.

Acceptable later LLM uses:

- query expansion;
- reranking a small candidate set;
- summarizing already-computed evidence;
- producing concise next-command suggestions.

Unacceptable default LLM uses:

- scanning the whole repo in a prompt;
- inventing architecture without evidence;
- claiming exact blast radius;
- replacing deterministic search.

### 4. Disposable CLI

Normal commands should run, answer, and exit.

No daemon should be required for MVP.
No file watcher should be required for MVP.
No background embedding job should be required for MVP.

A later `agentgrep index` command is allowed because it is explicit, local, and exits. It should create a lightweight cache of repository facts, not a resident service.

### 5. Low-token output

Agentgrep should not dump the whole repo into the agent context. It should return the smallest useful answer.

Prefer:

- top 5 to 10 candidates;
- concise reasons;
- line ranges;
- confidence;
- next commands.

### 6. Honest uncertainty

Blast radius is an estimate. The tool should report confidence and evidence, not certainty.

Good:

```text
Risk: medium
Confidence: low
Reason: dynamic imports may hide references; no test map found.
```

Bad:

```text
Only these files are impacted.
```

## MVP scope

The first working version should implement only the smallest useful spine:

```bash
agentgrep find "query"
agentgrep find "query" --json
```

MVP `find` should:

- run inside a git repo or normal directory;
- shell out to `rg` if available;
- search code, docs, configs, and tests;
- preserve `rg` as the recall floor;
- run exact-phrase and token search where useful;
- report candidate and match coverage when results are limited;
- rank file candidates;
- include reasons for ranking;
- show small snippets or line ranges;
- support stable JSON output;
- stay fast on small and medium repositories.

## Near-term command roadmap

After MVP `find`, add commands in this order:

```bash
agentgrep index
agentgrep index --status
agentgrep map <path>
agentgrep connections <path>
agentgrep symbol <name>
agentgrep related <path|symbol>
agentgrep blast <path|symbol>
agentgrep tests <path|symbol>
agentgrep plan "task prompt"
```

`index` should not require a budget flag. It should run fast by default, skip expensive work, and report what it indexed. Missing or stale index data must not block `find`; `rg` remains the recall floor.

## Command intent

### `find`

Find likely files or symbols for a query.

Example:

```bash
agentgrep find "auth redirect"
```

Should answer:

```text
Start here:
1. src/auth/session.ts
   Reason: path match, symbol match, route proximity.
2. src/routes/login.ts
   Reason: contains redirect logic and imports auth session.
```

### `index`

Build a lightweight local repository-facts cache.

Example:

```bash
agentgrep index
```

Should collect cheap deterministic facts such as:

```text
files, roles, symbols, imports, exports, file connections, tests, package/build hints
```

It should not create a daemon, watcher, vector database, or embedding store.

### `map`

Show a compact local map around a file or subsystem.

Example:

```bash
agentgrep map app/meeting_session.py
```

Should answer:

```text
File role: meeting lifecycle coordinator
Symbols: MeetingSession, start, stop
Imports: audio_recording, storage, schemas
Imported by: routers/meeting_sessions.py
Calls/references: storage.write_event, audio_recording.start
Likely tests: tests/test_meeting_sessions.py
```

### `connections`

Show direct file-level connections.

Example:

```bash
agentgrep connections app/meeting_session.py
```

Should answer:

```text
Outgoing:
- app/audio_recording.py — imported
- app/storage.py — imported / referenced

Incoming:
- app/routers/meeting_sessions.py — imports target file
- tests/test_meeting_sessions.py — tests target behavior
```

### `symbol`

Find definitions and references for a symbol.

Example:

```bash
agentgrep symbol MeetingSession
```

### `blast`

Estimate risk and likely impacted files/tests.

Example:

```bash
agentgrep blast app/meeting_session.py
```

### `tests`

Suggest relevant tests to run first.

Example:

```bash
agentgrep tests app/audio_recording.py
```

## Success criteria

Agentgrep is successful if it improves coding-agent workflows compared with plain `rg`.

The important metrics are:

- p50 and p95 latency;
- top-k file localization;
- precision of suggested files;
- false-positive burden;
- usefulness of reasons;
- JSON schema stability;
- downstream agent success;
- token savings versus raw search output.

## Non-goals for early versions

Do not build these until the core CLI proves value:

- embeddings;
- vector database;
- daemon;
- watcher;
- web UI;
- graph visualization;
- hosted service;
- cloud LLM dependency;
- repo-wide AI summaries;
- automatic code edits.

## Long-term vision

Agentgrep should become the standard local codebase radar for coding agents.

The mature version should help an agent move through this workflow quickly:

```text
classify task
-> generate search seeds
-> find likely edit locus with rg-backed recall
-> use lightweight index facts when available
-> map local structure and file connections
-> estimate blast radius
-> select tests/checks
-> return concise evidence
```

It should feel like giving the agent better eyes, not giving it another brain.
