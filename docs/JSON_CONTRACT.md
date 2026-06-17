# JSON Contract

Agentgrep JSON is v0.1.

The contract is intended to stay stable during v0.x unless this document says otherwise. Additive fields may appear. Best-effort fields may be null, omitted, or expanded as heuristics improve.

## General rules

- Stable fields are the fields agents and scripts should depend on.
- Best-effort fields provide extra context and may change more often.
- `score` is only meaningful within one command response.
- `score` is not comparable across commands or versions.
- `confidence` is a coarse label: `low`, `medium`, or `high`.
- `evidence` is explainability metadata and may grow over time.
- `index_status` reports the index state for the current command.
- `index_used` means index facts contributed to `find` ranking or evidence.
- `risk_level` is the conservative blast estimate: `low`, `medium`, or `high`.
- `coverage.semantic_status` reports whether semantic retrieval was active for a `find` response. Current values: `not_requested` (default, no `--semantic` flag passed). Future value `active` when a configured provider is used. Semantic mode is experimental and opt-in only via `--semantic`.

## `index_status`

Current values:

- `not_applicable`: `find` did not use an index.
- `missing`: no index file was available.
- `fresh`: the index exists and matches the current repo revision.
- `stale`: the index exists, but the repo revision changed.
- `unverifiable`: the repo revision could not be checked.

## `find --json`

Example:

```bash
agentgrep find "SearchResult" --json
```

Top-level shape:

```json
{
  "query": "string",
  "repo_root": "string",
  "repo_rev": "string | null",
  "latency_ms": 0,
  "coverage": {
    "raw_rg_match_count": 0,
    "raw_candidate_file_count": 0,
    "displayed_candidate_count": 0,
    "limited": false,
    "match_limit_per_file": 0,
    "candidate_limit": 0,
    "index_used": false,
    "index_status": "not_applicable",
    "semantic_status": "not_requested"
  },
  "candidates": [],
  "next_actions": []
}
```

Stable fields:

- `query`
- `repo_root`
- `repo_rev`
- `latency_ms`
- `coverage`
- `candidates`
- `next_actions`

Best-effort fields:

- `coverage.raw_rg_match_count`
- `coverage.raw_candidate_file_count`
- `coverage.displayed_candidate_count`
- `coverage.limited`
- `coverage.match_limit_per_file`
- `coverage.candidate_limit`
- `coverage.index_used`
- `coverage.index_status`
- `coverage.semantic_status`
- candidate `snippets`
- candidate `evidence`

Candidate fields:

- `path`
- `kind`
- `role`
- `score`
- `confidence`
- `line_ranges`
- `snippets`
- `evidence`

Notes:

- `score` is a relative ranking number for this response only.
- `confidence` is a coarse label, not a probability.
- `evidence` explains why the candidate was ranked and may gain new items in future versions.

## `map --json`

Example:

```bash
agentgrep map src/search.rs --json
```

Top-level shape:

```json
{
  "path": "string",
  "role": "string",
  "index_status": "string",
  "index_path": "string",
  "repo_rev": "string | null",
  "size_bytes": 0,
  "modified_unix": 0,
  "content_hash": "string | null",
  "symbols": [],
  "outgoing_edges": [],
  "incoming_edges": [],
  "connection_counts": {},
  "next_actions": []
}
```

Stable fields:

- `path`
- `role`
- `index_status`
- `index_path`
- `repo_rev`
- `size_bytes`
- `modified_unix`
- `content_hash`
- `symbols`
- `outgoing_edges`
- `incoming_edges`
- `connection_counts`
- `next_actions`

Best-effort fields:

- `repo_rev` when the repo revision is unavailable
- `size_bytes` when the file cannot be read
- `modified_unix` when the file timestamp is unavailable
- `content_hash` when the file is too large or unavailable

Notes:

- `symbols`, `outgoing_edges`, and `incoming_edges` are deterministic index-derived facts when the index is fresh.
- Unknown metadata is represented as `null` instead of inventing a value.

## `symbol --json`

Example:

```bash
agentgrep symbol SearchResult --json
```

Top-level shape:

```json
{
  "query": "string",
  "index_status": "string",
  "match_mode": "exact | case_insensitive | substring | none",
  "matches": [],
  "next_actions": []
}
```

Stable fields:

- `query`
- `index_status`
- `match_mode`
- `matches`
- `next_actions`

Best-effort fields:

- `matches` content when the index is missing or incomplete

Notes:

- `match_mode` tells you how exact the symbol lookup was.
- `matches` may be empty when the index is missing or no symbol match exists.

## `related --json`

Example:

```bash
agentgrep related src/search.rs --json
```

Top-level shape:

```json
{
  "query": "string",
  "mode": "file | symbol",
  "index_status": "string",
  "match_mode": "exact | case_insensitive | substring | none",
  "target_file": "string | null",
  "target_role": "string | null",
  "symbol_matches": [],
  "related_files": [],
  "edges": [],
  "symbols": [],
  "references": [],
  "next_actions": []
}
```

Stable fields:

- `query`
- `mode`
- `index_status`
- `related_files`
- `edges`
- `symbols`
- `references`
- `next_actions`

Best-effort fields:

- `match_mode`
- `target_file`
- `target_role`
- `symbol_matches`

Notes:

- `symbol_matches` is omitted when it is empty in the current serializer.
- `related_files` are ranked with a relative `score` that is only meaningful within this response.
- `confidence` on related files is a coarse label.

## `blast --json`

Example:

```bash
agentgrep blast src/search.rs --json
```

Top-level shape:

```json
{
  "query": "string",
  "mode": "file | symbol",
  "index_status": "string",
  "risk_level": "low | medium | high",
  "risk_reasons": [],
  "impacted_files": [],
  "affected_symbols": [],
  "references": [],
  "suggested_inspection_order": [],
  "next_actions": []
}
```

Stable fields:

- `query`
- `mode`
- `index_status`
- `risk_level`
- `risk_reasons`
- `impacted_files`
- `affected_symbols`
- `references`
- `suggested_inspection_order`
- `next_actions`

Best-effort fields:

- `risk_reasons`
- impacted file `score`
- impacted file `confidence`
- impacted file `context`
- impacted file `reasons`

Notes:

- `risk_level` is a conservative estimate, not a guarantee.
- `score` on impacted files is only comparable within this response.
- `confidence` stays coarse and should not be treated as exact probability.

## `--semantic` flag (experimental, opt-in)

`find --semantic` and `index --semantic` are active in this release.

Provider: **fastembed**, model **BAAI/bge-small-en-v1.5** (384 dimensions, CPU-only, no GPU required).

### Workflow

```bash
# Step 1: build the normal index (required first)
agentgrep index

# Step 2: build the semantic index (prompts for ~130 MB model download on first run)
agentgrep index --semantic
# or accept silently in CI / scripts:
agentgrep index --semantic --yes

# Step 3: semantic-expanded find
agentgrep find "where is the embedding provider configured" --semantic
```

### Behavior

- `coverage.semantic_status` is `"not_requested"` (default) or `"active"` when `--semantic` was used and succeeded.
- Evidence entries of type `"semantic_match"` appear in candidate evidence when semantic contributed. Detail format: `"cosine 0.NNN (BAAI/bge-small-en-v1.5)"`.
- Semantic candidates are always labeled separately from deterministic evidence.
- Deterministic (rg + BM25 + graph) evidence is always stronger than semantic evidence in the final ranking.
- Default `find` behavior (no `--semantic`) is unchanged.
- If the semantic index is missing or stale, `find --semantic` fails with a clear error and the next action: `agentgrep index --semantic`.

### Storage

- Model files: platform cache dir (Windows `%LOCALAPPDATA%\agentgrep\models\`; macOS/Linux `~/.cache/agentgrep/models/`). Global, not per-repo.
- Semantic index: `.git/agentgrep/semantic/` (git repos) or `.agentgrep/semantic/` (non-git). Contains `meta.json` and `vectors.bin`.

Do not depend on semantic evidence being present. It is additive and opt-in only. See `docs/SEMANTIC.md` for full documentation.

