# Agentgrep: Codex Agent Instructions

Practical instructions for Codex-style terminal/code agents.

## Core rules

- Prefer short, targeted commands over broad sweeps.
- Use `agentgrep find` before opening files manually when the target is unknown.
- Use `agentgrep blast` before editing any file with non-trivial connections.
- Redirect long outputs to `manual-test/` rather than printing them inline.
- Do not run full-repo sweeps unless no narrower search is possible.
- Do not assume semantic search is available — it is not.

## When to reach for Agentgrep

Use Agentgrep (not raw `rg`) when:
- you need ranked file candidates for an open-ended query;
- you need structural context (symbols, edges, impact) before editing;
- you want `--json` for stable programmatic output.

Use `rg` directly when:
- you know the exact string;
- you are piping to `awk`, `sed`, or another tool;
- you need raw lines with no ranking overhead.

## Preferred command patterns

### Localize before opening

```bash
agentgrep find "query term" --json
```

Read the top `candidates` in the JSON. Open only the top-ranked files.

### Build index when needed

```bash
agentgrep index --status
agentgrep index
```

Run once per session when structural commands are needed. Skip if already fresh.

### Inspect a candidate file

```bash
agentgrep map src/target.rs --json
```

Use `symbols` and `edges` from the output to decide what to read. Do not read the whole file first.

### Trace a symbol

```bash
agentgrep symbol SymbolName --json
```

Check `definitions` and `used_by`. Treat test-only references as lower priority.

### Check neighbors

```bash
agentgrep related src/target.rs --json
```

Review high-confidence related files before editing.

### Estimate impact

```bash
agentgrep blast src/target.rs --json
```

Inspect `risk_level` and `suggested_inspection_order`. Do not edit without reviewing medium/high-risk results.

## Output handling

If any command produces output too long to process inline:

```bash
agentgrep find "wide query" > manual-test/find-output.txt
agentgrep blast src/large.rs --json > manual-test/blast-output.json
```

Reference the saved file instead of re-running.

## What not to do

- Do not run `agentgrep find` with a very broad single-word query and read all results.
- Do not assume blast output is exhaustive — files not listed may still be affected.
- Do not skip `agentgrep blast` before editing widely-imported files.
- Do not compare scores across different queries or commands.
- Do not treat confidence values as probabilities.

## System prompt snippet

Add this to your Codex system prompt to register Agentgrep as a tool:

```
Available local tools:
- agentgrep find "<query>" [--json]  ranked file search over the codebase
- agentgrep index [--status]         build or check the local code index
- agentgrep map <file> [--json]      file-level symbol and edge context
- agentgrep symbol <name> [--json]   definitions and references for a name
- agentgrep related <file> [--json]  connected files by edges
- agentgrep blast <file> [--json]    conservative change impact estimate

Use agentgrep find before opening files.
Use agentgrep blast before editing files with broad connections.
Always use --json when parsing output programmatically.
Do not assume semantic search is available.
```

## Future work

Evaluation of prompt strategies across real codebase tasks is planned but not yet complete.
