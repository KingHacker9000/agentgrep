# Agentgrep: Generic Agent Instructions

Agentgrep is a local, evidence-first CLI for codebase navigation.
It ranks files by relevance, provides symbol and graph context, and estimates change impact.
It has no LLM, daemon, database, or semantic search. Evidence is deterministic and local.

## When to use `rg` vs `agentgrep`

Use `rg` when:
- you know the exact string and want raw match lines;
- you are piping output to another tool;
- you need pure grep speed with no ranking.

Use `agentgrep find` when:
- you want ranked file candidates, not raw lines;
- you want structured JSON for downstream use (`--json`);
- you want multi-term coverage (`--match all`);
- you want file-role filtering (`--role source`);
- you want `next_actions` to guide follow-up.

For config files, docs, or simple string searches, `rg` is usually sufficient.
Use `agentgrep` when file ranking or structural context matters.

## No-index mode vs indexed mode

### No-index mode

`agentgrep find` works with `rg` only — no index required.

Limitations without an index:
- `map`, `symbol`, `related`, `blast` have limited or no graph context;
- `find` ranking uses only lexical signals.

Suitable for: first contact with a repo, quick single-term lookup, error message search.

### Indexed mode

Run `agentgrep index` once to unlock structural context.

With the index:
- `find` gains symbol-name boosts and graph context;
- `map` shows incoming/outgoing edges;
- `symbol` reports definitions and references;
- `related` uses import/reference edges;
- `blast` gives a more precise impact estimate.

Check freshness before using structural commands:

```bash
agentgrep index --status
```

Rebuild if stale or missing.

## Command chains

### Unknown feature: localize from scratch

```bash
agentgrep find "feature term"
agentgrep map <top-result-file>
agentgrep related <top-result-file>
```

Use `find` to narrow candidates, `map` to inspect the likely file, `related` to see its neighborhood.

### Symbol tracing

```bash
agentgrep index
agentgrep symbol <SymbolName>
agentgrep map <definition-file>
```

Use `index` first, then `symbol` to locate definitions and usages, then `map` for full context.

### Impact check before editing

```bash
agentgrep blast <file-or-symbol>
agentgrep related <file-or-symbol>
agentgrep map <impacted-file>
```

Use `blast` to estimate conservative likely impact, `related` to see connected files, `map` to inspect specifics before editing.

### Config or docs search

```bash
rg "option-name" docs/ config/
agentgrep find "option-name" --role config
```

Prefer `rg` for raw config/doc search. Use `agentgrep find --role config` when file ranking across many config files helps.

## Evidence and citation rules

- Always cite specific file paths and line numbers from Agentgrep output when referencing code.
- Use `Why` evidence and `next_actions` fields to explain ranking decisions.
- Do not claim a file is relevant without citing at least one ranked result or evidence signal.
- Do not claim a file is safe to change based solely on blast output — blast is a conservative estimate, not a guarantee.
- Confidence values (`low | medium | high`) indicate inspection priority, not correctness.
- Scores are relative within a single response; do not compare across commands or queries.

## Future work

Evaluation of prompt strategies across real codebase tasks is planned but not yet complete.
Claims about agent prompt effectiveness will require empirical measurement.
