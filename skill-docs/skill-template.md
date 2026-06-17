# Agentgrep Skill Template

Use this template to define an Agentgrep skill for a specific coding agent or workflow.
Copy, fill in the sections, and trim what does not apply.

---

## Skill: [Name]

**Purpose:** [One sentence describing what this skill helps the agent accomplish.]

**When to use:**
- [Scenario 1]
- [Scenario 2]

**When not to use:**
- [Scenario where a simpler tool like `rg` is sufficient]
- [Scenario where this skill does not apply]

---

## Commands

### Find

```bash
agentgrep find "<query>" [--json] [--match all] [--role source]
```

Use to locate ranked file candidates for a query.
Prefer `--json` when parsing output programmatically.

### Index

```bash
agentgrep index
agentgrep index --status
```

Run once before using `map`, `symbol`, `related`, or `blast`.
Check freshness with `--status`.

### Map

```bash
agentgrep map <file> [--json]
```

Use after identifying a candidate file.
Provides symbols, incoming edges (callers), outgoing edges (dependencies), and next actions.

### Symbol

```bash
agentgrep symbol <SymbolName> [--json]
```

Use to find definitions and references for a named symbol.
Reports production vs test usage context.

### Related

```bash
agentgrep related <file-or-symbol> [--json]
```

Use to find files connected by imports, references, or symbol relationships.
Inspect high-confidence results first.

### Blast

```bash
agentgrep blast <file-or-symbol> [--json]
```

Use before editing to estimate conservative likely impact.
Inspect `risk_level` and `suggested_inspection_order`.

---

## Recommended workflow

```bash
# Step 1: Localize
agentgrep find "<task terms>" --json

# Step 2: Build index if needed
agentgrep index --status
agentgrep index

# Step 3: Inspect candidate file
agentgrep map <top-file> --json

# Step 4: Trace symbol if relevant
agentgrep symbol <SymbolName> --json

# Step 5: Check neighborhood
agentgrep related <file-or-symbol> --json

# Step 6: Estimate impact
agentgrep blast <file-or-symbol> --json

# Step 7: Read, edit, test
```

---

## Output interpretation

| Field | How to use |
|---|---|
| `candidates` | Ranked file list — read top results first |
| `why` / evidence | Explains ranking — cite in your reasoning |
| `next_actions` | Suggested follow-up commands |
| `confidence` | Inspection priority (`low / medium / high`), not a probability |
| `risk_level` | Blast estimate (`low / medium / high`), not a guarantee |
| `suggested_inspection_order` | Order to inspect impacted files |

Scores are relative within one response only. Do not compare across queries or commands.

---

## Agent-specific notes

[Fill in any agent-specific behavior, output handling, or restrictions.]

Examples:
- Redirect long outputs to `manual-test/` rather than processing inline.
- Use `--json` for all tool calls that will be parsed programmatically.
- Do not assume semantic search is available.

---

## What not to do

- Do not skip `agentgrep blast` before editing widely-imported files.
- Do not treat blast output as exhaustive.
- Do not claim evidence without citing a file path and line range from output.
- Do not compare scores across commands or query runs.

---

## Related docs

- [generic-agent.md](./generic-agent.md) — neutral instructions for any agent
- [claude.md](./claude.md) — Claude Code-specific instructions
- [codex.md](./codex.md) — Codex-style agent instructions
- [docs/AGENTS.md](../docs/AGENTS.md) — full command reference and JSON consumption rules
- [docs/JSON_CONTRACT.md](../docs/JSON_CONTRACT.md) — stable JSON contract
