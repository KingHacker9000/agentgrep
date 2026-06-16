# Agentgrep Roadmap

## Current milestone checklist

- [x] MVP core command loop
- [x] Release hardening
- [x] JSON contract stabilization
- [ ] Dogfood on real repos
- [ ] Config file
- [ ] Retrieval v2: BM25 / FTS / identifier expansion / graph boosts
- [ ] Tree-sitter Rust backend
- [ ] Optional hybrid semantic mode behind a flag
- [ ] Multi-language support
- [ ] Packaging / integrations

## Current status

Agentgrep has completed its v0.1-style core.

Available commands:

```text
find -> index -> map -> symbol -> related -> blast
```

The project also has:

- polished README;
- CI for format/check/test;
- Windows smoke script;
- JSON contract documentation;
- top-level JSON shape tests.

The next phase is dogfooding on real repositories.

## Roadmap principles

### 1. Keep the default path lightweight

Agentgrep should stay a fast local CLI.

Default behavior should not require:

- daemon;
- watcher;
- database server;
- dashboard;
- resident model;
- background service.

### 2. `rg` remains the recall floor

Agentgrep is not a replacement for `rg`.

For search, `rg` should remain the first recall layer. Agentgrep adds ranking, evidence, snippets, symbols, edges, references, and next actions.

### 3. JSON is a first-class agent interface

Agents should be able to depend on stable top-level JSON fields.

The JSON contract is documented in:

```text
docs/JSON_CONTRACT.md
```

### 4. Add intelligence without hidden resource cost

Prefer deterministic local upgrades before model-based upgrades.

The order is:

```text
BM25/FTS + identifier expansion + graph boosts
before
optional hybrid semantic retrieval
```

### 5. Semantic retrieval is opt-in only

Hybrid semantic search may be valuable later, but only if it remains:

- disabled by default;
- local-first;
- explicit through a flag;
- non-resident;
- no required GPU;
- clearly labeled in output.

### 6. Dogfood before widening scope

Before adding parser and retrieval complexity, Agentgrep should be tested on messy real repositories.

Failures from real repos should drive the next quality pass.

---

# Completed milestones

## Milestone 0 — Product foundation

Status: complete.

Agentgrep is defined as:

```text
fast local code radar for coding agents
```

Core constraints:

- local;
- disposable;
- CLI-native;
- evidence-backed;
- agent-shaped;
- lightweight;
- no LLM in core;
- no daemon or background service.

## Milestone 1 — MVP core command loop

Status: complete.

Completed commands:

| Command | Purpose |
|---|---|
| `find <query>` | Evidence-first search for likely files. |
| `index` | Build or inspect the lightweight repository index. |
| `map <path>` | Inspect one file with indexed context. |
| `symbol <name>` | Find definitions and references for a symbol. |
| `related <path-or-symbol>` | Inspect nearby files, symbols, edges, and references. |
| `blast <path-or-symbol>` | Estimate conservative likely impact. |

Exit state:

- full command loop works;
- index is optional;
- `find` works without index;
- graph-aware commands use index and give useful next actions.

## Milestone 2 — Release hardening

Status: complete.

Completed:

- polished GitHub-style README;
- crate metadata in `Cargo.toml`;
- GitHub Actions CI;
- Windows smoke script;
- help-output captures;
- clear release checklist.

Exit state:

- repo can be pushed publicly;
- CI passes;
- local smoke script passes;
- README explains command workflow and non-goals.

## Milestone 3 — JSON contract stabilization

Status: complete.

Completed:

- `docs/JSON_CONTRACT.md`;
- README link to JSON contract;
- stable top-level JSON field tests for:
  - `find`;
  - `map`;
  - `symbol`;
  - `related`;
  - `blast`.

Exit state:

- v0.1 JSON contract is documented;
- stable vs best-effort fields are separated;
- score/confidence/evidence/index/risk semantics are documented.

---

# Remaining milestones

## Milestone 4 — Dogfood on real repos

Status: next.

Goal:

Test Agentgrep against real repositories to find practical ranking, evidence, JSON, and workflow issues.

Suggested repos:

- Agentgrep itself;
- ReMeet;
- QuickGet CLI;
- QuickGet desktop/Tauri;
- QuickGet browser extension.

Dogfood checklist for each repo:

```bash
agentgrep index
agentgrep find "known feature or symbol"
agentgrep map <important-file>
agentgrep symbol <known-symbol>
agentgrep related <file-or-symbol>
agentgrep blast <file-or-symbol>
```

Collect:

- wrong top result;
- noisy evidence;
- missing symbol;
- bad next action;
- confusing risk level;
- stale or missing index behavior;
- JSON contract pain;
- slow command;
- large-output problem.

Exit criteria:

- at least 2 real repos tested;
- concrete issue list produced;
- next quality pass selected from evidence, not guesses.

## Milestone 5 — Config file

Status: planned.

Goal:

Add a small `.agentgrep.toml` for practical repo-level tuning.

Possible config:

```toml
[output]
candidate_limit = 8
edge_limit = 5
reference_limit = 5

[index]
exclude = ["target", "node_modules", "dist", ".venv"]

[ranking]
demote_tests = true
prefer_source = true
```

Rules:

- keep config optional;
- keep defaults good;
- no complex plugin system;
- no background service;
- no behavior surprises.

Exit criteria:

- config discovery works from repo root;
- output caps can be adjusted;
- index excludes can be adjusted;
- tests cover missing/default config.

## Milestone 6 — Retrieval v2: BM25 / FTS / identifier expansion / graph boosts

Status: planned.

Goal:

Make `find` stronger without ML.

This is the main default intelligence upgrade.

Possible capabilities:

- local FTS/BM25-style indexed text search;
- identifier expansion;
- camelCase / snake_case token expansion;
- filename/path token boosts;
- symbol-name expansion;
- graph-aware boosts;
- better query rewriting without LLM.

Example:

```text
query: "where do we handle missing ripgrep"

should find:
- "rg was not found"
- run_rg
- search.rs
- install ripgrep error handling
```

Rules:

- keep `rg` as recall floor;
- no embeddings by default;
- no LLM;
- no daemon;
- no database server;
- local-only;
- deterministic evidence remains primary.

Exit criteria:

- `find` improves on conceptual wording without model calls;
- FTS/BM25 evidence is clearly labeled;
- indexed lexical candidates do not swamp exact symbol/text matches;
- JSON contract remains stable at top level.

## Milestone 7 — Tree-sitter Rust backend

Status: planned.

Goal:

Improve Rust indexing accuracy.

Use Tree-sitter inside indexing for:

- symbol extraction;
- imports;
- references where practical;
- line ranges;
- skipping comments;
- reducing fixture/string false positives.

Rules:

- Rust first;
- no command UX change;
- heuristics remain fallback;
- no daemon;
- no schema explosion;
- no attempt to become a compiler.

Exit criteria:

- Rust symbol extraction becomes more accurate;
- fixture/comment/string false positives decrease;
- existing graph commands keep working;
- tests compare parser-backed output against key fixtures.

## Milestone 8 — Optional hybrid semantic mode behind a flag

Status: planned, later.

Goal:

Offer deeper search on larger codebases without changing the default lightweight path.

Possible shape:

```bash
agentgrep index --semantic
agentgrep find --semantic "where is auth state restored"
```

or:

```bash
agentgrep find --deep "where is auth state restored"
```

Rules:

- disabled by default;
- explicit flag required;
- local-only;
- no always-running model;
- no required GPU;
- no server;
- no hidden background work;
- semantic candidates are labeled;
- deterministic evidence still wins final ranking.

Exit criteria:

- semantic layer is optional and removable;
- default install remains lightweight;
- semantic results are clearly distinguished from deterministic evidence.

## Milestone 9 — Multi-language support

Status: planned.

Goal:

Extend index and graph usefulness beyond Rust.

Likely order:

1. TypeScript / JavaScript;
2. Python;
3. Go;
4. Markdown/docs cross-links.

Start lightweight:

- imports;
- exports;
- functions;
- classes;
- types;
- tests;
- references where easy.

Rules:

- no compiler-grade ambition;
- confidence labels matter;
- language-specific extraction must degrade gracefully;
- default output stays concise.

Exit criteria:

- Agentgrep is useful on at least one mixed-language repo;
- language extraction improves `map`, `symbol`, `related`, and `blast`;
- missing language support does not break generic search.

## Milestone 10 — Packaging / integrations

Status: planned.

Goal:

Make Agentgrep easier to install and integrate.

Possible work:

- release binaries;
- `cargo install --git` instructions;
- versioned release notes;
- shell completions;
- editor snippets;
- Codex/Claude usage guide;
- optional MCP adapter after JSON contract proves stable.

Rules:

- CLI remains primary;
- JSON remains the integration surface;
- MCP/editor integration should not create a daemon requirement.

Exit criteria:

- fresh user can install and run Agentgrep quickly;
- agents can call it using documented JSON;
- releases are repeatable.

---

# Explicitly deferred

These are intentionally not current roadmap work:

- LLM in core;
- always-on embedding service;
- background indexing daemon;
- dashboard;
- database server;
- full language server;
- exact compiler-grade dependency graph;
- exact blast-radius guarantees.

They can be revisited only if real dogfooding shows a strong need and the lightweight CLI model remains intact.
