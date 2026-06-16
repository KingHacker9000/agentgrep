<div align="center">

# Agentgrep

fast local code radar for coding agents.

[![Rust](https://img.shields.io/badge/rust-stable-CE422B?logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![CI](https://img.shields.io/badge/CI-GitHub_Actions-2088FF?logo=githubactions&logoColor=white)](./.github/workflows/ci.yml)
[![No LLM](https://img.shields.io/badge/LLM-none-2E7D32)](#philosophy--non-goals)
[![Local-first](https://img.shields.io/badge/local--first-yes-0F766E)](#philosophy--non-goals)

</div>

Agentgrep is a local, evidence-first CLI that helps coding agents decide where to look next.

## What is Agentgrep?

Agentgrep is a small Rust CLI for codebase navigation.

It uses `rg` as the recall floor, then groups matches by file, ranks the likely candidates, and adds lightweight local evidence when available.

The index is optional. If it is missing, `find` still works with `rg` only.

There is no LLM, daemon, watcher, background service, or remote dependency in the core workflow.

## Why not just `rg`?

`rg` is still the fastest way to collect raw matches.

Agentgrep sits on top of that recall floor and makes the first pass more agent-shaped:

- it groups matches by file;
- it ranks likely files with explainable heuristics;
- it keeps line numbers and snippets attached;
- it can use an optional local index to improve ranking and context;
- it can print stable JSON for downstream tools.

If you just need raw search, `rg` is still the right tool.

## Install

Build from source with stable Rust:

```bash
cargo install --path .
```

For local development:

```bash
cargo build
```

## Quick start

```bash
agentgrep find "query"
agentgrep find "query" --json
agentgrep index
```

Useful follow-ups:

```bash
agentgrep map <file>
agentgrep symbol <name>
agentgrep related <file-or-symbol>
agentgrep blast <file-or-symbol>
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

Current work stays focused on the deterministic local radar:

- better `find` ranking and evidence;
- optional lightweight indexing;
- file maps;
- symbol awareness;
- related-file and blast-radius heuristics;
- test recommendations;
- only then, optional semantic or hybrid retrieval as an explicit opt-in.

Semantic or hybrid retrieval is future work only. It should remain optional and local-first if it is added at all.

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

## Release checklist

Before a release, run:

```bash
cargo fmt
cargo check
cargo test
powershell -ExecutionPolicy Bypass -File scripts/smoke.ps1
```

If you need to inspect long help or report output, capture it into `manual-test/`.
