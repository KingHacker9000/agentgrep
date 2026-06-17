# Agentgrep Semantic Mode

> **Experimental and opt-in.** Semantic mode does not affect default behavior. Pass `--semantic` explicitly to use it.

---

## Overview

Agentgrep's semantic mode adds file-level embedding search on top of the deterministic pipeline. When enabled, a query is embedded and compared against pre-computed file vectors using cosine similarity. The semantic candidates are merged with deterministic candidates (rg + BM25 + graph evidence) and clearly labeled.

Semantic mode is:

- disabled by default;
- activated by explicit `--semantic` flag;
- local-only (no cloud API);
- CPU-only (no GPU required);
- non-resident (no daemon or background service);
- file-level only in Phase 1 (no chunk-level embeddings).

---

## Provider

| Property | Value |
|---|---|
| Library | [fastembed](https://github.com/Anush008/fastembed-rs) |
| Model | BAAI/bge-small-en-v1.5 |
| Dimensions | 384 |
| Model size | ~130 MB (one-time download) |
| Runtime | ONNX Runtime (CPU) |
| GPU required | No |

---

## Workflow

```bash
# Step 1: build the normal index
agentgrep index

# Step 2: build the semantic index
#   On first run: prompts "Download embedding model (~130 MB)? [y/N]"
agentgrep index --semantic

#   CI / scripts: skip the prompt
agentgrep index --semantic --yes

# Step 3: semantic-expanded find
agentgrep find "where is the embedding provider configured" --semantic
agentgrep find "SearchResult" --semantic --json
```

---

## File storage

### Model cache (global, not per-repo)

Downloaded once per machine.

| Platform | Path |
|---|---|
| Windows | `%LOCALAPPDATA%\agentgrep\models\` |
| macOS / Linux | `~/.cache/agentgrep/models/` (or `$XDG_CACHE_HOME/agentgrep/models/`) |

A sentinel file `.agentgrep-model-ready` is written after the first successful download. Subsequent runs load from cache without prompting.

### Semantic index (per-repo)

| Repo type | Path |
|---|---|
| Git repo | `.git/agentgrep/semantic/` |
| Non-git repo | `.agentgrep/semantic/` |

Files written:

| File | Contents |
|---|---|
| `meta.json` | Schema version, provider, model, dimensions, agentgrep version, repo rev, created_at, per-file list with hashes |
| `vectors.bin` | Raw f32 vectors (little-endian binary: u32 count, u32 dims, then count×dims×4 bytes) |

The semantic index is separate from `index.json` and is never required for default operation.

---

## meta.json schema

```json
{
  "schema_version": 1,
  "provider": "fastembed",
  "model_name": "BAAI/bge-small-en-v1.5",
  "dimensions": 384,
  "agentgrep_version": "0.1.4",
  "repo_rev": "abc1234",
  "index_stamp": 1718000000,
  "created_at": 1718000010,
  "files": [
    { "path": "src/semantic.rs", "content_hash": "deadbeef" },
    ...
  ]
}
```

---

## Document representation

Each file is embedded as a short text document:

```
path: src/semantic.rs
role: source
symbols: SemanticState, ensure_model, expand_candidates
---
<first 1500 characters of file content>
```

The header gives context for the file's identity and declared symbols. The content gives semantic signal. Binary files or files that cannot be read produce an empty content section but are still indexed with their header.

---

## Find --semantic merge behavior

1. Deterministic candidates are computed first (rg + BM25 + graph evidence), ranked by deterministic score.
2. The query is embedded.
3. Brute-force cosine search over all file vectors; candidates with similarity ≥ 0.30 are selected (up to 8).
4. Merge:
   - **File already in deterministic results**: `semantic_match` evidence is appended; score receives a small boost (`+similarity × 0.3`).
   - **Semantic-only file** (not found by rg): added as a new candidate with score `similarity × 0.8`, confidence `low` (or `medium` if similarity ≥ 0.60).
5. Re-ranked by combined score; capped at `CANDIDATE_LIMIT` (8).
6. `coverage.semantic_status` is set to `"active"`.

Deterministic evidence always dominates: a file with strong rg/BM25/graph signal scores much higher than a semantic-only match.

---

## JSON contract changes

When `--semantic` is active:

```json
{
  "coverage": {
    "semantic_status": "active"
  },
  "candidates": [
    {
      "evidence": [
        {
          "type": "semantic_match",
          "detail": "cosine 0.712 (BAAI/bge-small-en-v1.5)"
        }
      ]
    }
  ]
}
```

When `--semantic` is not passed: `coverage.semantic_status` is `"not_requested"` and no `semantic_match` evidence appears. This is the default.

---

## Staleness

The semantic index stores `repo_rev` at index time. On `find --semantic`, if the current repo revision differs from the stored one, the command fails with:

```
semantic index is stale (indexed at abc1234, current rev def5678).
Run `agentgrep index --semantic` to refresh it.
```

If either revision is unavailable (non-git repo, no HEAD), the index is used as-is.

---

## Limitations (Phase 1)

- **File-level only.** One vector per indexed file; no chunk-level embeddings. Long files are truncated at 1500 characters for the document.
- **English-biased model.** BAAI/bge-small-en-v1.5 is optimized for English prose. Code identifier names often work well; heavily non-English comments may produce weaker results.
- **No evaluation baseline yet.** Mode D has not been formally compared against Mode C on real repos. Results are plausible but unquantified. See `docs/evaluation/README.md`.
- **Brute-force search.** O(n) cosine scan over all file vectors. Fast for repos with <10 000 files; acceptable for Phase 1.
- **Cosine threshold is fixed at 0.30.** May surface weakly related files. Adjust threshold expectations accordingly.
- **Model load time.** Loading the ONNX model takes 1–5 seconds per invocation. There is no resident model or caching across CLI calls.

---

## Managing the semantic index

```bash
# Show the state of the semantic index and model cache
agentgrep semantic status

# Remove only the repo-local semantic index (meta.json + vectors.bin)
agentgrep semantic clean --repo-index

# Remove only the global model cache (~130 MB download)
agentgrep semantic clean --model

# Remove both
agentgrep semantic clean --all
```

`status` shows:
- Semantic index path and whether it exists
- Model name and dimensions (if the index is readable)
- File count in the index
- Model cache path and whether it is present

`clean` operations are explicit and do not prompt. Removing the semantic index does not affect the normal index or any other Agentgrep functionality. Removing the model cache means the next `agentgrep index --semantic` will re-download the model.
