# Agentgrep Roadmap

## Purpose

This roadmap defines the broad capability milestones Agentgrep should reach before it is ready for serious real-world testing with coding models and agentic workflows.

This is not an implementation checklist. It describes what should become possible at each stage and what kind of product capability each milestone unlocks.

Agentgrep's north star remains:

```text
A fast, disposable, evidence-first CLI that gives coding agents better local codebase evidence: where to look, what depends on what, what might break, and what to run next.
```

## Roadmap principles

### 1. Build the radar before testing the pilot

Agentgrep should first become a reliable deterministic codebase radar. Only after that should it be tested deeply with LLMs and coding agents.

Do not add model-based features before the core search, evidence, mapping, blast-radius, and test-selection capabilities are useful without models.

### 2. Capabilities over infrastructure

Each milestone should unlock a new useful capability for agents.

Avoid treating infrastructure itself as progress. A daemon, database, watcher, vector store, dashboard, or LLM integration is not valuable unless it directly improves the agent workflow.

### 3. Evidence before summaries

Agentgrep should return evidence that an agent can reason over. Summaries are useful only when they are grounded in already-collected evidence.

### 4. Real-world testing comes after workflow coverage

Testing with coding agents becomes meaningful only after Agentgrep supports the common navigation workflow:

```text
find -> index -> map/connections -> symbol -> related/trace -> blast -> tests -> plan
```

## Milestone overview

| Milestone | Capability unlocked | Main question answered |
|---|---|---|
| 0. Product foundation | Clear identity and constraints | What is Agentgrep, and what is it not? |
| 1. Better-than-rg search | Ranked search with reasons | Where should the agent look first? |
| 2. Agent-readable evidence | Stable, concise, parseable output | Can agents consume results reliably? |
| 2.5. Recall contract | rg-backed coverage guarantees | Did find account for what rg would have found? |
| 2.6. Lightweight index | Fast local repository facts and file connections | What does this repo connect to without heavy infrastructure? |
| 3. Local file maps | Tactical file/subsystem understanding | What is around this file? |
| 4. Symbol awareness | Definitions and references | Where is this thing defined and used? |
| 5. Dependency and relationship awareness | Local codebase structure | What depends on what? |
| 6. Blast radius v1 | Risk and impact estimation | What might break if this changes? |
| 7. Test recommendation | Validation targeting | What should the agent run next? |
| 8. Task planning mode | First-pass workflow guidance | How should the agent begin this task? |
| 9. Evaluation harness | Measured usefulness | Is this better than plain rg? |
| 10. Optional LLM assist | Bounded model help | Does a small model improve results enough? |
| 11. Agent integration | Real agent workflow usage | Can Codex/Claude-style agents use it naturally? |
| 12. Real-world beta | Messy repo validation | Would we actually keep this installed? |

---

# Milestone 0 — Product foundation

## Goal

Agentgrep has a clear product identity, scope, and constraint set.

## Capabilities

Agentgrep should have docs that explain:

- what the tool is;
- who it is for;
- why it exists;
- what problems it solves;
- what it intentionally does not solve;
- how coding agents should use it;
- what early versions should not overbuild.

## Product meaning

This milestone prevents drift into the wrong product category.

Agentgrep should not become:

- a repo chatbot;
- a semantic-search SaaS;
- an always-on code intelligence server;
- an embedding database wrapper;
- a dashboard-first graph explorer.

## Exit criteria

This milestone is complete when the project has at least:

```text
PROJECT.md
ARCHITECTURE.md
AGENTS.md
ROADMAP.md
```

and those docs consistently describe Agentgrep as:

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

---

# Milestone 1 — Better-than-rg search

## Goal

Agentgrep becomes useful as a smarter search command.

## Capabilities

Agentgrep can answer:

```text
Where should I look first for this query or task?
```

It should support queries like:

```bash
agentgrep find "auth redirect"
agentgrep find "meeting lifecycle"
agentgrep find "audio recording"
```

The tool can:

- run lexical search;
- group matches by file;
- rank likely files;
- surface line ranges;
- explain why each candidate appears;
- avoid overwhelming output;
- suggest next useful commands.

## Product meaning

This is the first point where Agentgrep can be compared against plain `rg`.

The tool does not need deep code intelligence yet. It only needs to produce a better first answer than raw grep output.

## Example output shape

```text
Top candidates:
1. app/meeting_session.py
   Why: path suggests meeting area; multiple lifecycle matches; likely coordinator file.

2. app/routers/meeting_sessions.py
   Why: route-like path; references meeting session operations.

Next:
- agentgrep map app/meeting_session.py
- agentgrep tests app/meeting_session.py
```

## Exit criteria

This milestone is complete when a coding agent would reasonably prefer:

```bash
agentgrep find "query"
```

over a raw `rg` call for initial localization.

---

# Milestone 2 — Agent-readable evidence system

## Goal

Agentgrep output becomes reliable enough for coding agents to consume directly.

## Capabilities

Each result can include:

- path;
- kind;
- score;
- confidence;
- line ranges;
- evidence list;
- short explanation;
- next commands;
- latency;
- repository revision when available.

Output should support both:

```text
human-readable text
machine-readable JSON
```

## Product meaning

This milestone turns Agentgrep from a nicer human CLI into an agent interface.

Agents need stable schemas and low-token results. They should not need to parse messy prose or huge command dumps.

## Exit criteria

This milestone is complete when:

- JSON output is stable enough for agents;
- evidence types are explicit;
- output is concise by default;
- every important candidate explains why it was selected;
- malformed or missing evidence is avoided.

---

# Milestone 2.5 — Recall contract

## Goal

Agentgrep `find` should preserve `rg` as the recall floor while still adding ranking, snippets, evidence, and agent-friendly structure.

## Capabilities

`find` can report:

- how many raw `rg` matches were collected;
- how many candidate files were found;
- how many candidates are displayed;
- whether the result was limited or truncated;
- whether index facts were used;
- whether the index was missing, fresh, stale, or partial.

## Product meaning

This milestone prevents Agentgrep from becoming a clever filter that hides relevant raw search results.

The promise is not:

```text
Agentgrep always ranks perfectly.
```

The promise is:

```text
Agentgrep starts from rg-visible evidence and tells the agent when results were limited.
```

## Exit criteria

This milestone is complete when an agent can tell from text or JSON output whether `find` saw the full raw search result set or only a limited subset.

---

# Milestone 2.6 — Lightweight index and file connections

## Goal

Agentgrep can build a fast local cache of deterministic repository facts that improves later commands without becoming a daemon, watcher, embedding service, or graph database.

## Capabilities

Agentgrep can support:

```bash
agentgrep index
agentgrep index --status
agentgrep index --clear
```

There should be no budget flag. `index` should be fast by default by doing cheap work only. On large repos, it may create a partial index and clearly report what was skipped.

The index can contain:

- file catalog;
- file roles;
- symbol definitions;
- imports and exports;
- direct file connections;
- likely tests;
- package/build hints;
- git revision and content hashes.

File connection edges can include:

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

## Product meaning

This is how Agentgrep starts competing with graph-style tools while staying lightweight.

The index should improve:

```text
find
map
connections
symbol
related
blast
tests
```

but missing or stale index data must never block `find`.

## Exit criteria

This milestone is complete when `agentgrep index` can quickly create a local cache and `agentgrep find` can use that cache as ranking/evidence while preserving `rg` recall.

---

# Milestone 3 — Local file maps

## Goal

Agentgrep can explain the immediate neighborhood around a file.

## Capabilities

Agentgrep can answer:

```text
What is this file, and what is around it?
```

Example command:

```bash
agentgrep map app/meeting_session.py
```

The map can include:

- file role guess;
- major functions/classes;
- imports;
- exports;
- incoming file connections;
- outgoing file connections;
- route-like declarations;
- config-like declarations;
- nearby tests;
- related files;
- suggested next files to inspect.

## Product meaning

This milestone gives agents a compact local mental model without reading many full files.

It is the first major step from search to codebase mapping.

## Exit criteria

This milestone is complete when an agent can take a found file and quickly understand:

```text
what it contains
what it connects to
what to inspect next
```

without manually running several separate shell commands.

---

# Milestone 4 — Symbol awareness

## Goal

Agentgrep understands symbols as first-class navigation objects.

## Capabilities

Agentgrep can answer:

```text
Where is this class/function/type defined?
Where is it referenced?
What file owns it?
What tests mention it?
```

Example commands:

```bash
agentgrep symbol MeetingSession
agentgrep symbol start_recording
agentgrep symbol AuthMiddleware
```

The tool can return:

- definitions;
- references;
- symbol kind;
- file locations;
- line ranges;
- likely callers;
- related tests;
- confidence when exactness is weak.

## Product meaning

Agents often think in symbols. This milestone lets Agentgrep operate at the same level as coding tasks.

Search answers where strings appear. Symbol awareness answers where code concepts live.

## Exit criteria

This milestone is complete when an agent can trace an important class/function/type through a repo without manually combining many `rg` calls.

---

# Milestone 5 — Dependency and relationship awareness

## Goal

Agentgrep can surface relationships between files and modules.

## Capabilities

Agentgrep can answer:

```text
What imports this?
What does this import?
What files are structurally nearby?
What files historically change with this one?
What code areas are related but not obvious from text search?
```

Example commands:

```bash
agentgrep trace app/meeting_session.py
agentgrep related app/audio_recording.py
```

The tool can surface:

- import edges;
- reverse imports;
- dependency chains;
- local module neighborhoods;
- historically co-changed files;
- recently touched files;
- high-churn or risky files.

## Product meaning

This milestone helps agents understand codebase shape, not just code locations.

It starts answering:

```text
What else is nearby?
What does this depend on?
What depends on this?
What might be related even if names differ?
```

## Exit criteria

This milestone is complete when Agentgrep can reveal useful adjacent files that plain text search would often miss.

---

# Milestone 6 — Blast radius v1

## Goal

Agentgrep can estimate what might be impacted by a change.

## Capabilities

Agentgrep can answer:

```text
If I change this file or symbol, what might break?
```

Example commands:

```bash
agentgrep blast app/meeting_session.py
agentgrep blast MeetingSession.start
agentgrep blast "change auth token refresh"
```

The blast report can include:

- risk level;
- confidence;
- likely impacted files;
- likely impacted tests;
- direct references;
- import/reverse-import evidence;
- public API hints;
- history/co-change evidence;
- known blind spots;
- suggested validation commands.

## Product meaning

This is one of the core reasons Agentgrep exists.

Coding agents can make narrow edits too confidently. Blast radius gives them a risk map before they modify code.

## Important constraint

Blast radius must be honest.

Good:

```text
Likely impacted files, ranked by evidence.
Confidence: medium.
Blind spots: dynamic imports not analyzed.
```

Bad:

```text
Only these files are impacted.
```

## Exit criteria

This milestone is complete when blast reports help agents avoid obvious missed ripple effects while still avoiding massive useless candidate sprawl.

---

# Milestone 7 — Test recommendation

## Goal

Agentgrep can suggest relevant validation commands.

## Capabilities

Agentgrep can answer:

```text
What tests or checks should I run after touching this?
```

Example command:

```bash
agentgrep tests app/audio_recording.py
```

The tool can suggest:

- direct tests;
- nearby tests;
- integration tests;
- smoke tests;
- package-level checks;
- missing-test warnings;
- confidence levels.

## Product meaning

This is extremely useful for real agentic coding because agents constantly need to choose validation scope.

A good test recommendation saves time while still catching likely failures.

## Exit criteria

This milestone is complete when Agentgrep can reduce wasted test running without hiding important tests from the agent.

---

# Milestone 8 — Task planning mode

## Goal

Agentgrep can help an agent begin a new task in an unfamiliar codebase.

## Capabilities

Agentgrep can answer:

```text
Given this task prompt, how should the agent start navigating?
```

Example command:

```bash
agentgrep plan "add botless screen capture to meetings"
```

The tool can return:

- likely query terms;
- likely files to inspect;
- likely symbols;
- suggested Agentgrep command sequence;
- likely edit areas;
- likely tests;
- early risk notes.

## Product meaning

This milestone supports the first phase of agent work: orientation.

It should not become a full coding agent. It should only produce a tactical navigation plan.

## Example output shape

```text
Suggested workflow:
1. agentgrep find "screen capture meeting screenshot frame"
2. agentgrep map app/meeting_session.py
3. agentgrep symbol MeetingSession
4. agentgrep blast app/meeting_session.py
5. agentgrep tests app/meeting_session.py
```

## Exit criteria

This milestone is complete when Agentgrep can give a useful first navigation plan for a fresh repo and task prompt.

---

# Milestone 9 — Evaluation harness

## Goal

Agentgrep can be measured against plain tools and agent workflows.

## Capabilities

The project can evaluate:

- Agentgrep vs plain `rg`;
- top-k file localization;
- precision of suggested candidates;
- false-positive burden;
- blast-radius usefulness;
- test recommendation quality;
- latency;
- output token savings;
- JSON schema stability;
- downstream agent success.

## Product meaning

This milestone prevents vibes-based development.

Agentgrep should prove that it improves coding-agent workflows, not merely that its output looks nice.

## Evaluation repos

Testing should include:

- small projects;
- medium projects;
- larger projects;
- typed-heavy repos;
- dynamic-language repos;
- messy real projects;
- projects with weak or missing tests.

## Exit criteria

This milestone is complete when the team can answer:

```text
Does Agentgrep help an agent localize, map, and validate changes faster than plain rg?
```

with measured evidence.

---

# Milestone 10 — Optional LLM assist

## Goal

Agentgrep can use a small model only where bounded model help is useful.

## Capabilities

Possible commands or flags:

```bash
agentgrep find "why does login bounce back" --llm
agentgrep find "auth redirect" --rerank
agentgrep blast app/session.py --summarize
```

Allowed model tasks:

- natural-language query expansion;
- repo-specific term suggestion;
- reranking top deterministic candidates;
- summarizing already-computed evidence;
- suggesting next commands.

Disallowed default model tasks:

- whole-repo semantic indexing;
- hidden repo-wide reasoning;
- exact blast-radius claims;
- ungrounded architecture summaries;
- background inference;
- sending repo contents to cloud without explicit opt-in.

## Product meaning

This milestone tests whether models improve Agentgrep after the deterministic radar already works.

The LLM is not the foundation. It is a bounded assistant on top of evidence.

## Exit criteria

This milestone is complete when evaluation shows whether a small local model improves results enough to justify its latency and complexity.

If not, Agentgrep should remain mostly deterministic.

---

# Milestone 11 — Agent integration

## Goal

Agentgrep becomes usable in real coding-agent workflows.

## Capabilities

Coding agents can naturally use:

```text
agentgrep find
agentgrep map
agentgrep symbol
agentgrep related
agentgrep blast
agentgrep tests
agentgrep plan
```

Agent integration should provide:

- clear command documentation;
- stable JSON schemas;
- predictable failure modes;
- concise outputs;
- good next-command suggestions;
- useful behavior without hand-holding.

## Product meaning

This is the first milestone where serious real-world testing with Codex/Claude-style agents makes sense.

The agent should be able to chain commands like:

```text
find -> map -> symbol -> blast -> tests
```

without needing custom explanation every time.

## Exit criteria

This milestone is complete when coding agents can use Agentgrep across multiple realistic tasks without special prompting or manual babysitting.

---

# Milestone 12 — Real-world beta

## Goal

Agentgrep is ready to be tested on messy real projects.

## Capabilities

Agentgrep should handle:

- real repo layouts;
- missing tests;
- weak docs;
- generated files;
- non-git folders;
- dynamic languages;
- medium-to-large repos;
- unclear user prompts;
- partial confidence;
- graceful degradation.

It should remain:

- fast enough;
- local-first;
- evidence-backed;
- concise;
- reliable for agents;
- honest about uncertainty.

## Product meaning

This milestone asks the real adoption question:

```text
Would we actually keep this installed and tell agents to use it before raw rg?
```

## Exit criteria

This milestone is complete when Agentgrep is useful enough that a coding agent's default workflow can include it naturally:

```text
Before reading random files or dumping rg output, use Agentgrep to localize, map, assess risk, and choose tests.
```

---

# Readiness ladder

A compact way to track readiness:

```text
0. Product foundation
1. Better-than-rg find
2. Evidence + JSON output
2.5. Recall contract
2.6. Lightweight index + file connections
3. Local file maps
4. Symbol awareness
5. Dependency/relationship awareness
6. Blast radius v1
7. Test recommendation
8. Task planning mode
9. Evaluation harness
10. Optional LLM assist
11. Agent integration
12. Real-world beta
```

## Final principle

Do not test with models too early.

First build the deterministic radar.

Then measure whether models become better pilots when they use it.
