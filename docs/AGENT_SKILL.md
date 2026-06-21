# agentgrep — Agent Skill Guide

agentgrep is a Rust CLI for evidence-first codebase search. No LLM, no daemon. Use it when
you need to locate code in a repo rather than read files you already know about.

## Commands at a glance

| Command | Use when | Input | Key output |
|---------|----------|-------|------------|
| `find <query>` | You need to locate files for a concept or identifier | natural language or identifier | ranked `candidates[]` with score, evidence, snippets |
| `symbol <name>` | You know an exact symbol name | identifier | definition location + `used_by[]` callers |
| `map <path>` | You have a file and want its symbol inventory | file path | symbols defined in the file |
| `related <path>` | You want neighbour files (imports, same area) | file path | import graph + co-located files |
| `blast <query>` | You need impact estimate before editing | file or symbol | blast radius + confidence |
| `peek <name>` | You want the body of a known symbol | symbol name | source lines from `line_number` to `end_line` |
| `index` | One-time setup or after large changes | — | builds `.agentgrep/index.json` |

## Decision tree

```
Do I know the exact symbol name?
├─ YES → agentgrep symbol <Name>
│         ├─ found: read definition line + used_by
│         └─ not found: run agentgrep index, then retry
│
└─ NO → agentgrep find "<concept or phrase>"
          ├─ score >= 0.70 (Full): read top candidate, likely done
          ├─ score 0.45-0.70 (Medium): read top 1-2, may need follow-up
          ├─ score < 0.45 (Minimal/Enum): concept is spread; try more specific query
          └─ no results: fall back to rg "<term>" for raw recall
```

## Query formulation rules

1. **Use the identifier as-is for symbol lookups**: `agentgrep symbol RequestContext` not
   `agentgrep find "request context"`. The symbol command handles case-insensitive exact +
   substring matching.

2. **Use noun phrases for topic queries**: `agentgrep find "middleware registration"` not
   `agentgrep find "app.use function"`. Strip verbs, articles, and code punctuation.

3. **Split camelCase mentally but keep it as one token**: `agentgrep find "downloadProgress"`
   — the tokenizer splits it. Do NOT write `agentgrep find "download progress"` unless you
   actually want the phrase.

4. **Specificity beats breadth**: `agentgrep find "session context push pop"` outperforms
   `agentgrep find "context"`. Include 2-4 key terms.

5. **Don't pad**: omit words like "function", "method", "class", "code", "implementation".

## Output interpretation

```json
{
  "candidates": [
    {
      "path": "src/rank.rs",
      "score": 0.85,
      "confidence": "high",
      "detail_level": "full",
      "evidence": [
        { "evidence_type": "indexed_symbol_definition", "detail": "..." },
        { "evidence_type": "exact_phrase_match", "detail": "..." }
      ],
      "snippets": [{ "line_number": 42, "text": "fn build_candidate(...)" }]
    }
  ]
}
```

- **score**: 0.0–1.0. Scores are calibrated: 0.70+ means strong multi-signal match.
- **confidence**: reflects evidence quality, NOT score alone. `high` = indexed definition +
  phrase match. `medium` = phrase or strong lexical. `low` = weak or single signal.
- **detail_level**: `full` (all fields) | `medium` (evidence trimmed to 2) | `minimal`
  (no snippets, 1 evidence) | `enum` (path+score only). Lower-ranked results are shown with
  less detail to keep output compact.
- **evidence**: ordered list, most relevant first. Key types:
  - `indexed_symbol_definition` — file defines the symbol you searched for (strongest)
  - `exact_phrase_match` — search phrase appears literally in the file
  - `filename_shape_match` — filename matches all query terms (reliable for topic queries)
  - `near_phrase_match` — terms cluster within 10 lines
  - `lexical_score` — BM25 term frequency match

## Typical workflows

### Locate a function definition
```bash
agentgrep symbol push              # exact name → definition + callers
agentgrep symbol RequestContext    # class/struct lookup
agentgrep peek push --file ctx.py  # show body if end_line is indexed
```

### Understand a feature area
```bash
agentgrep find "session context management"   # topic query → file list
# read top 1-2 files, then:
agentgrep related src/session.rs              # neighbour files
agentgrep map src/session.rs                  # symbol inventory
```

### Pre-edit impact check
```bash
agentgrep blast RequestContext     # what breaks if I change this?
agentgrep symbol push              # who calls push?
```

### Fall-through to rg (when index is stale or missing)
```bash
agentgrep find "error handler"     # if score < 0.30 across all results:
rg "error_handler|handle_error" --type py -l  # raw grep for recall
```

## Anti-patterns

| Wrong | Right | Why |
|-------|-------|-----|
| `agentgrep find "RequestContext.push"` | `agentgrep symbol push --file ctx.py` | Dot-notation is not a query term |
| `agentgrep find "function that handles middleware"` | `agentgrep find "middleware handler"` | Verbose descriptions hurt precision |
| `agentgrep find "code"` | `agentgrep find "<actual concept>"` | Trivially matches everything |
| Skip index rebuild after checkout | `agentgrep index` | Stale index degrades symbol scores |
| Read 10 Enum-tier files | Re-query with narrower terms | Enum tier means low signal |

## Index management

```bash
agentgrep index          # build or rebuild
agentgrep index --status # check freshness (fresh/stale/missing)
agentgrep index --clear  # remove index
```

The index is stored in `.agentgrep/index.json`. Rebuild after: major refactors, branch
switches with large diffs, or when symbol command returns unexpected "not found".

## Score thresholds (reference)

| Score | Detail level | Recommended action |
|-------|-------------|-------------------|
| ≥ 0.70 | Full | Read file; likely the answer |
| 0.45–0.70 | Medium | Read top 1-2; may need follow-up |
| 0.25–0.45 | Minimal | Cross-reference with rg or broaden query |
| < 0.25 | Enum | Path only; treat as breadcrumb not answer |
