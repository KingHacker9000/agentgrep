<div align="center">

# Agentgrep

fast local code radar for coding agents.

[![Rust](https://img.shields.io/badge/rust-stable-CE422B?logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![CI](https://img.shields.io/badge/CI-GitHub_Actions-2088FF?logo=githubactions&logoColor=white)](./.github/workflows/ci.yml)
[![No LLM](https://img.shields.io/badge/LLM-none-2E7D32)](#philosophy--non-goals)
[![Local-first](https://img.shields.io/badge/local--first-yes-0F766E)](#philosophy--non-goals)

</div>

Agentgrep is a local, evidence-first CLI that helps coding agents decide where to look next.

Coding agents spend context budget on repository navigation. Plain tools like `rg` return raw matches — the agent still has to rank files, trace symbols, and guess at impact. Agentgrep does that work instead: it groups results by file, ranks candidates with explainable heuristics, adds symbol and graph evidence from a lightweight optional index, and prints stable JSON an agent can act on directly. No LLM, no daemon, no background service.

## What is Agentgrep?

Agentgrep is a small Rust CLI for codebase navigation.

It uses `rg` as the recall floor, then groups matches by file, ranks the likely candidates, and adds lightweight local evidence when available.

The index is optional. If it is missing, `find` still works with `rg` only.

There is no LLM, daemon, watcher, background service, or remote dependency in the core workflow.

## Why not just `rg`?

`rg` is still the right tool for raw exact line search, and Agentgrep keeps it as the recall floor.

Agentgrep adds a layer on top for agent workflows:

- groups matches by file so agents rank files, not lines;
- ranks candidates with explainable heuristics (path tokens, symbol presence, graph edges);
- supports `--match any` and `--match all` for broad or strict coverage;
- `--include`, `--exclude`, and `--role` narrow results without manual grep chaining;
- bare globs like `*.css` match by basename anywhere; `src/**/*.css` stays path-specific;
- keeps line numbers and snippets attached to each file result;
- optional index adds symbol context, inbound/outbound edges, and file roles;
- `--json` prints a stable machine-readable shape agents can consume directly.

**The practical split:** use `rg` when you know exactly what string you want. Use Agentgrep when an agent needs ranked files, evidence, symbol context, relationships, JSON output, and impact hints.

## Install

Install from GitHub (no local clone needed):

```bash
cargo install --git https://github.com/KingHacker9000/agentgrep
```

Or build from a local clone:

```bash
cargo install --path .
```

For local development only (no install):

```bash
cargo build
```

Requirements: Rust stable (1.75+) and `rg` on PATH. See [docs/INSTALL.md](./docs/INSTALL.md) for full install and verification steps.

**Docs:** [Install guide](./docs/INSTALL.md) · [Release checklist](./docs/RELEASE.md) · [JSON contract](./docs/JSON_CONTRACT.md) · [Agent skill docs](./skill-docs/) · [Evaluation scaffold](./docs/evaluation/) · [CHANGELOG](./CHANGELOG.md)

## Quick start

The core navigation loop:

```bash
agentgrep find "auth redirect"         # rank files by evidence
agentgrep map src/auth.rs              # inspect one file in context
agentgrep symbol AuthHandler           # find definitions and callers
agentgrep related src/auth.rs          # see nearby files and edges
agentgrep blast src/auth.rs            # estimate likely impact before editing
```

Run `agentgrep index` once in a repo to unlock symbol extraction, graph edges, and better ranking across all commands.

Other useful `find` flags:

```bash
agentgrep find "query" --match all
agentgrep find "query" --include "*.css"
agentgrep find "query" --role source
agentgrep find "query" --json
```

## Command workflow

The usual flow is:

```text
find -> index -> map -> symbol -> related -> blast
```

Practical examples:

```bash
agentgrep find "auth redirect"
agentgrep index
agentgrep map src/search.rs
agentgrep symbol SearchResult
agentgrep related src/search.rs
agentgrep blast src/search.rs
```

## JSON output

Use `--json` on commands that support it when you need stable machine-readable output:

```bash
agentgrep find "auth redirect" --json
agentgrep map src/search.rs --json
agentgrep symbol SearchResult --json
agentgrep related src/search.rs --json
agentgrep blast src/search.rs --json
```

JSON is intended for agents and scripts. Text output stays concise for terminal use.

JSON contract details live in [docs/JSON_CONTRACT.md](./docs/JSON_CONTRACT.md).

## Index behavior

`agentgrep index` is optional.

It improves ranking and context when present, but it never replaces `rg` as the recall floor.

The index is stored locally in the repo's git area when available, otherwise in `.agentgrep/index.json`.

`agentgrep index --status` reports whether the cache is fresh, stale, missing, or unverifiable.

## Example agent workflow

1. `agentgrep find "auth redirect"` to localize the likely files.
2. `agentgrep index` if you want better file connections and symbol context.
3. `agentgrep map src/search.rs` to inspect one file in context.
4. `agentgrep symbol SearchResult` to see definitions and references.
5. `agentgrep related src/search.rs` to inspect nearby files and connections.
6. `agentgrep blast src/search.rs` to estimate conservative likely impact before editing.

`blast` is a conservative likely-impact estimate. It is not a guarantee of breakage.

## Roadmap

The v0.1-style deterministic core is complete. Active development is on evaluation infrastructure and reliability improvements.

Completed work:

- core command loop (`find`, `index`, `map`, `symbol`, `related`, `blast`);
- retrieval v2: BM25-style lexical ranking, identifier expansion, graph boosts;
- confidence-aware fusion for Mode C/D ranking;
- Tree-sitter multi-language indexing (Rust, Python, JS/TS, Go);
- comparative evaluation scaffold with labeled task sets and regression gates;
- optional semantic mode (`--semantic`) via fastembed — disabled by default, local-only.

Next: dogfood on real repos, config file.

## What Agentgrep is and is not

Agentgrep is an **agent-shaped code radar** that sits on top of fast local search. It is not a replacement for `rg`.

| Tool | Best for |
|---|---|
| `rg` | Exact raw line search across a repo — fastest, no setup, no ranking needed |
| `agentgrep find` | Agent needs ranked file candidates with evidence and optional JSON |
| `agentgrep map/symbol/related/blast` | Agent needs local context, symbol graph, or impact estimate before editing |

`rg` and Agentgrep are complementary. Use `rg` inside Agentgrep's recall floor; use Agentgrep when the agent needs ranked, explained, structured output.

## Philosophy / non-goals

Agentgrep is designed to be:

- local;
- disposable;
- CLI-native;
- evidence-backed;
- agent-shaped;
- lightweight;
- honest about uncertainty.

Agentgrep is not:

- an LLM wrapper;
- a daemon;
- a background service;
- a file watcher;
- a database server;
- a dashboard;
- a semantic search engine;
- a replacement for `rg`.

## Agent skill docs

Instructions and workflow templates for using Agentgrep in agentic coding environments:

- [skill-docs/generic-agent.md](./skill-docs/generic-agent.md) — neutral instructions usable by any coding agent
- [skill-docs/claude.md](./skill-docs/claude.md) — Claude Code-style agent loops and examples
- [skill-docs/codex.md](./skill-docs/codex.md) — Codex-style terminal agent instructions
- [skill-docs/skill-template.md](./skill-docs/skill-template.md) — reusable skill template for other agents

See also: [docs/AGENTS.md](./docs/AGENTS.md) for the full command reference.

## Release checklist

Before a release, run:

```bash
cargo fmt --all -- --check
cargo check --all-targets
cargo test --all-targets
powershell -ExecutionPolicy Bypass -File scripts/smoke.ps1
cargo run -- --version
```

If you need to inspect long help or report output, capture it into `manual-test/`.

See [docs/RELEASE.md](./docs/RELEASE.md) for the full release procedure.
