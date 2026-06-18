# Semantic Backend — Developer Notes

Developer-facing measurements, dependency impact, and implementation notes for the semantic mode backend.

For user-facing documentation, see [docs/SEMANTIC.md](../SEMANTIC.md).

---

## Dependency impact

fastembed 4.x pulls a substantial dependency tree.

| Metric | Value (approximate) |
|---|---|
| New packages added to Cargo.lock | ~293 |
| Notable transitive deps | `ort` (ONNX Runtime), `tokenizers`, `hf-hub`, `reqwest`, `image`, `rav1e` |
| fastembed version resolved | 4.9.1 |

Most of the weight comes from:
- `ort` — ONNX Runtime static linking (~largest single contributor to binary size increase)
- `tokenizers` — HuggingFace tokenizer library (Rust port)
- `reqwest` — used by `hf-hub` for model downloads

These are transitive; Agentgrep only directly depends on `fastembed = "4"`.

---

## Binary size reference

> Measure with: `cargo build --release` then check the `.exe` or binary size.

Not yet measured after fastembed was added. Expected increase: significant (tens of MB) due to ONNX Runtime static linking.

Before semantic: measure `cargo build --release` on the `main` branch.
After semantic: measure on this branch.

Record the delta here once measured.

---

## Model cache size reference

After the first `agentgrep index --semantic` run:

| Item | Approximate size |
|---|---|
| ONNX model file (BAAI/bge-small-en-v1.5) | ~66 MB |
| Tokenizer files | ~5 MB |
| Total model cache | ~130 MB |

Path: `%LOCALAPPDATA%\agentgrep\models\` (Windows), `~/.cache/agentgrep/models/` (macOS/Linux).

The sentinel file `.agentgrep-model-ready` is written after the first successful load.

---

## Semantic index size reference

For Agentgrep itself (small Rust repo, ~30–50 source files):

| File | Approximate size |
|---|---|
| `meta.json` | ~5–15 KB |
| `vectors.bin` | ~40–60 KB (n × 384 × 4 bytes) |

Formula: `vectors.bin` = `(n_files × 384 dims × 4 bytes per f32)` + 8 byte header.

For a repo with 1 000 files: ~1.5 MB. For 5 000 files: ~7.5 MB.

---

## Timing notes

> Measure with a stopwatch on `agentgrep index --semantic` after the model is cached.

| Operation | Expected range |
|---|---|
| Model load (ONNX init) | 1–5 seconds |
| Embedding 50 files | < 1 second |
| Embedding 500 files | 5–15 seconds |
| Embedding 5 000 files | 1–3 minutes |
| `find --semantic` (cosine scan, 500 files) | < 100 ms |

Model load dominates on every invocation. There is no resident model or cross-call cache.

---

## Constants (centralized in `src/semantic.rs`)

| Constant | Value | Purpose |
|---|---|---|
| `SEMANTIC_SCHEMA_VERSION` | 1 | Breaks compatibility on format changes |
| `PROVIDER` | `"fastembed"` | Recorded in meta.json |
| `MODEL_NAME` | `"BAAI/bge-small-en-v1.5"` | Recorded and checked on load |
| `EMBEDDING_DIMS` | 384 | Checked against meta.json on load |
| `TEXT_PREVIEW_CHARS` | 1500 | Characters of file content per document |
| `SENTINEL_FILE` | `.agentgrep-model-ready` | Written after first successful model load |
| `SEMANTIC_TOP_K` | 8 | Max cosine hits returned |
| `COSINE_THRESHOLD` | 0.30 | Minimum similarity to include a candidate |
| `FILE_WARN_THRESHOLD` | 5 000 | Warn when repo has more files (slow run) |
| `FILE_HARD_CAP` | 50 000 | Fail if repo exceeds this (memory safety) |

---

## Upgrading the model

To switch models (e.g., to a larger model or fastembed 5.x):

1. Update `MODEL_NAME` and `EMBEDDING_DIMS` in `src/semantic.rs`.
2. Update `EmbeddingModel::BGESmallENV15` in `init_model()` to the new model variant.
3. Bump `SEMANTIC_SCHEMA_VERSION` if the vector format changes (it usually does not).
4. Users will need to re-run `agentgrep index --semantic` — the model compatibility check in `load_semantic()` will detect the mismatch and print a clear error.

---

## vectors.bin binary format

```
u32  num_vectors    (little-endian)
u32  dimensions     (little-endian)
[num_vectors × dimensions × f32]  (each f32 little-endian)
```

Total size: `8 + num_vectors × dimensions × 4` bytes.

---

## Identifier-like query heuristic

`is_identifier_like(query)` returns `true` when:
- Query contains no whitespace, AND
- Query contains an uppercase letter after the first character (CamelCase), OR contains an underscore (snake_case)

When true, `expand_candidates` skips score boosting and does not inject semantic-only candidates. This keeps deterministic ranking primary for exact symbol lookups like `SearchResult` or `run_rg`.

When false (natural language: "where is auth state restored"), full merging is applied.

---

## Phase 1 limitations

- One vector per file (no chunk-level embeddings). Long files are truncated at 1500 chars.
- Brute-force O(n) cosine scan. Use Phase 2 ANN index for repos > 10 000 files.
- Model load is 1–5 seconds per CLI invocation. No resident model.
- Cosine threshold 0.30 may surface weakly related files.

See [docs/SEMANTIC.md](../SEMANTIC.md) for user-facing limitations.
