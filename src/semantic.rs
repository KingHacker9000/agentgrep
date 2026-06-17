/// Semantic retrieval backend — Milestone 8.
///
/// Storage paths:
///   git repos:     .git/agentgrep/semantic/  (meta.json + vectors.bin)
///   non-git repos: .agentgrep/semantic/
///
/// Model cache (global, not per-repo):
///   Windows:      %LOCALAPPDATA%/agentgrep/models/
///   macOS/Linux:  ~/.cache/agentgrep/models/   (XDG_CACHE_HOME honored)
///
/// Provider: fastembed, model BAAI/bge-small-en-v1.5 (384 dims, CPU-only).
use anyhow::{bail, Context, Result};
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::index::RepoIndex;
use crate::repo::{display_path, RepoInfo};
use crate::types::{Confidence, Evidence, FileCandidate};

// ── constants ─────────────────────────────────────────────────────────────────

pub const SEMANTIC_SCHEMA_VERSION: u32 = 1;
pub const PROVIDER: &str = "fastembed";
pub const MODEL_NAME: &str = "BAAI/bge-small-en-v1.5";
pub const EMBEDDING_DIMS: usize = 384;

/// Characters of file content included in the embedded document.
const TEXT_PREVIEW_CHARS: usize = 1500;

/// Sentinel written to the model cache dir after the first successful download.
const SENTINEL_FILE: &str = ".agentgrep-model-ready";

/// Maximum semantic candidates returned by cosine search.
const SEMANTIC_TOP_K: usize = 8;

/// Minimum cosine similarity to include a semantic candidate.
const COSINE_THRESHOLD: f32 = 0.30;

// ── legacy availability types (kept for tests; no longer called from main) ───

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SemanticState {
    Available,
}

#[allow(dead_code)]
pub fn check_availability() -> SemanticState {
    SemanticState::Available
}

#[allow(dead_code)]
pub fn require_configured(_subcommand: &str) -> Result<()> {
    Ok(())
}

// ── metadata types ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticMeta {
    pub schema_version: u32,
    pub provider: String,
    pub model_name: String,
    pub dimensions: usize,
    pub agentgrep_version: String,
    pub repo_rev: Option<String>,
    /// Proxy for which normal index was used: the indexed_at_unix timestamp.
    pub index_stamp: Option<u64>,
    pub created_at: u64,
    pub files: Vec<SemanticFileEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticFileEntry {
    pub path: String,
    pub content_hash: Option<String>,
}

pub struct SemanticIndex {
    pub meta: SemanticMeta,
    pub vectors: Vec<Vec<f32>>,
}

pub struct SemanticBuildReport {
    pub semantic_dir: PathBuf,
    pub file_count: usize,
    pub model_name: String,
    pub dimensions: usize,
    pub model_cache_dir: PathBuf,
}

// ── platform paths ────────────────────────────────────────────────────────────

pub fn model_cache_dir() -> Result<PathBuf> {
    #[cfg(windows)]
    {
        let base = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .ok_or_else(|| anyhow::anyhow!("LOCALAPPDATA environment variable is not set"))?;
        return Ok(base.join("agentgrep").join("models"));
    }

    #[cfg(not(windows))]
    {
        let base = if let Some(xdg) = std::env::var_os("XDG_CACHE_HOME") {
            PathBuf::from(xdg)
        } else {
            let home = std::env::var_os("HOME")
                .ok_or_else(|| anyhow::anyhow!("HOME environment variable is not set"))?;
            PathBuf::from(home).join(".cache")
        };
        return Ok(base.join("agentgrep").join("models"));
    }
}

pub fn semantic_dir(repo: &RepoInfo) -> PathBuf {
    match &repo.git_dir {
        Some(git_dir) => git_dir.join("agentgrep").join("semantic"),
        None => repo.root.join(".agentgrep").join("semantic"),
    }
}

// ── model management ──────────────────────────────────────────────────────────

fn is_model_cached(cache_dir: &Path) -> bool {
    cache_dir.join(SENTINEL_FILE).exists()
}

fn prompt_download_consent() -> Result<bool> {
    print!(
        "Semantic indexing requires a local embedding model (~130 MB one-time download).\n\
         Download now? [y/N]: "
    );
    io::stdout().flush().ok();
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    let answer = line.trim().to_lowercase();
    Ok(answer == "y" || answer == "yes")
}

fn init_model(cache_dir: &Path) -> Result<TextEmbedding> {
    TextEmbedding::try_new(
        InitOptions::new(EmbeddingModel::BGESmallENV15)
            .with_cache_dir(cache_dir.to_path_buf())
            .with_show_download_progress(true),
    )
    .context("failed to initialize embedding model")
}

/// Ensure the model is present and return a ready instance.
/// On first run, prompts for consent (or accepts via `yes`).
pub fn ensure_model(cache_dir: &Path, yes: bool) -> Result<TextEmbedding> {
    if !is_model_cached(cache_dir) {
        let proceed = if yes {
            eprintln!(
                "--yes: downloading embedding model to {}",
                display_path(cache_dir)
            );
            true
        } else if io::stdin().is_terminal() {
            prompt_download_consent()?
        } else {
            bail!(
                "embedding model not downloaded; re-run with --yes to accept the download:\n  \
                 agentgrep index --semantic --yes"
            );
        };

        if !proceed {
            bail!(
                "download declined — semantic indexing requires the model.\n  \
                 Re-run: agentgrep index --semantic"
            );
        }
    }

    fs::create_dir_all(cache_dir).with_context(|| {
        format!(
            "failed to create model cache dir at {}",
            display_path(cache_dir)
        )
    })?;

    eprintln!("Loading embedding model (first load may take a moment)...");
    let model = init_model(cache_dir)?;

    // Write sentinel so future runs skip the prompt.
    let sentinel = cache_dir.join(SENTINEL_FILE);
    if !sentinel.exists() {
        let _ = fs::write(&sentinel, b"");
    }

    Ok(model)
}

/// Load the model for `find --semantic`. Never prompts or downloads.
pub fn load_model_for_find(cache_dir: &Path) -> Result<TextEmbedding> {
    if !is_model_cached(cache_dir) {
        bail!(
            "embedding model not found at {}.\n\
             Run `agentgrep index --semantic` first to download the model and build \
             the semantic index.",
            display_path(cache_dir)
        );
    }
    init_model(cache_dir).with_context(|| {
        format!(
            "failed to load embedding model from {}",
            display_path(cache_dir)
        )
    })
}

// ── document building ─────────────────────────────────────────────────────────

fn build_file_document(
    path: &str,
    role: &str,
    symbols: &[String],
    content_preview: &str,
) -> String {
    let mut doc = String::with_capacity(256 + content_preview.len());
    doc.push_str("path: ");
    doc.push_str(path);
    doc.push('\n');
    doc.push_str("role: ");
    doc.push_str(role);
    doc.push('\n');
    if !symbols.is_empty() {
        doc.push_str("symbols: ");
        doc.push_str(&symbols.join(", "));
        doc.push('\n');
    }
    doc.push_str("---\n");
    doc.push_str(content_preview);
    doc
}

fn read_file_preview(file_path: &Path) -> String {
    let Ok(mut file) = fs::File::open(file_path) else {
        return String::new();
    };
    // Read slightly more than TEXT_PREVIEW_CHARS to account for multi-byte chars.
    let mut buf = vec![0u8; TEXT_PREVIEW_CHARS * 3];
    let n = file.read(&mut buf).unwrap_or(0);
    buf.truncate(n);
    let s = match std::str::from_utf8(&buf) {
        Ok(s) => s,
        Err(e) => match std::str::from_utf8(&buf[..e.valid_up_to()]) {
            Ok(s) => s,
            Err(_) => return String::new(),
        },
    };
    s.chars().take(TEXT_PREVIEW_CHARS).collect()
}

// ── binary vector storage ─────────────────────────────────────────────────────
//
// Format (little-endian):
//   u32  num_vectors
//   u32  dimensions
//   [num_vectors × dimensions × f32]

fn vectors_path(sem_dir: &Path) -> PathBuf {
    sem_dir.join("vectors.bin")
}

fn meta_path(sem_dir: &Path) -> PathBuf {
    sem_dir.join("meta.json")
}

fn write_vectors(path: &Path, vectors: &[Vec<f32>]) -> Result<()> {
    let n = vectors.len() as u32;
    let d = if vectors.is_empty() {
        EMBEDDING_DIMS as u32
    } else {
        vectors[0].len() as u32
    };

    let mut bytes: Vec<u8> = Vec::with_capacity(8 + n as usize * d as usize * 4);
    bytes.extend_from_slice(&n.to_le_bytes());
    bytes.extend_from_slice(&d.to_le_bytes());
    for vec in vectors {
        for f in vec {
            bytes.extend_from_slice(&f.to_le_bytes());
        }
    }

    fs::write(path, &bytes)
        .with_context(|| format!("failed to write vectors to {}", display_path(path)))
}

fn read_vectors(path: &Path) -> Result<Vec<Vec<f32>>> {
    let bytes = fs::read(path)
        .with_context(|| format!("failed to read vectors from {}", display_path(path)))?;

    if bytes.len() < 8 {
        bail!("vectors file is too short: {}", display_path(path));
    }

    let n = u32::from_le_bytes(bytes[0..4].try_into().unwrap()) as usize;
    let d = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
    let expected = 8 + n * d * 4;

    if bytes.len() < expected {
        bail!(
            "vectors file truncated (expected {} bytes, got {}): {}",
            expected,
            bytes.len(),
            display_path(path)
        );
    }

    let mut vectors = Vec::with_capacity(n);
    let mut offset = 8usize;
    for _ in 0..n {
        let mut vec = Vec::with_capacity(d);
        for _ in 0..d {
            let f = f32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
            vec.push(f);
            offset += 4;
        }
        vectors.push(vec);
    }

    Ok(vectors)
}

// ── semantic index build ──────────────────────────────────────────────────────

/// Build and persist the semantic index.
///
/// Requires a fresh normal index to be loaded and passed in; use
/// `index::load()` to obtain it. The model is downloaded on first run
/// (prompt or `--yes`).
pub fn build_semantic(
    repo: &RepoInfo,
    normal_index: Option<&RepoIndex>,
    yes: bool,
) -> Result<SemanticBuildReport> {
    let cache_dir = model_cache_dir()?;
    let sem_dir = semantic_dir(repo);

    let Some(idx) = normal_index else {
        bail!(
            "no normal index available.\n\
             Run `agentgrep index` first, then `agentgrep index --semantic`."
        );
    };

    let model = ensure_model(&cache_dir, yes)?;

    // Build a symbol lookup: file_path → Vec<symbol_name>
    let mut symbol_lookup: std::collections::HashMap<&str, Vec<String>> =
        std::collections::HashMap::new();
    for sym in &idx.symbols {
        symbol_lookup
            .entry(sym.file_path.as_str())
            .or_default()
            .push(sym.name.clone());
    }

    eprintln!("Embedding {} files...", idx.files.len());

    let mut file_entries: Vec<SemanticFileEntry> = Vec::with_capacity(idx.files.len());
    let mut documents: Vec<String> = Vec::with_capacity(idx.files.len());

    for file in &idx.files {
        let abs_path = repo.root.join(&file.path);
        let preview = read_file_preview(&abs_path);
        let symbols = symbol_lookup
            .get(file.path.as_str())
            .cloned()
            .unwrap_or_default();
        let role = file.role.to_string();
        documents.push(build_file_document(&file.path, &role, &symbols, &preview));
        file_entries.push(SemanticFileEntry {
            path: file.path.clone(),
            content_hash: file.content_hash.clone(),
        });
    }

    let embeddings: Vec<Vec<f32>> = model.embed(documents, None).context("embedding failed")?;

    let meta = SemanticMeta {
        schema_version: SEMANTIC_SCHEMA_VERSION,
        provider: PROVIDER.to_string(),
        model_name: MODEL_NAME.to_string(),
        dimensions: EMBEDDING_DIMS,
        agentgrep_version: env!("CARGO_PKG_VERSION").to_string(),
        repo_rev: repo.rev.clone(),
        index_stamp: Some(idx.indexed_at_unix),
        created_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        files: file_entries,
    };

    fs::create_dir_all(&sem_dir).with_context(|| {
        format!(
            "failed to create semantic directory at {}",
            display_path(&sem_dir)
        )
    })?;

    let meta_json = serde_json::to_string_pretty(&meta).context("failed to serialize meta")?;
    fs::write(meta_path(&sem_dir), &meta_json).context("failed to write meta.json")?;
    write_vectors(&vectors_path(&sem_dir), &embeddings)?;

    let file_count = meta.files.len();
    Ok(SemanticBuildReport {
        semantic_dir: sem_dir,
        file_count,
        model_name: MODEL_NAME.to_string(),
        dimensions: EMBEDDING_DIMS,
        model_cache_dir: cache_dir,
    })
}

pub fn write_semantic_build_report(report: &SemanticBuildReport) {
    println!("Semantic index written:");
    println!("- files embedded: {}", report.file_count);
    println!("- model: {}", report.model_name);
    println!("- dimensions: {}", report.dimensions);
    println!("- semantic dir: {}", display_path(&report.semantic_dir));
    println!("- model cache:  {}", display_path(&report.model_cache_dir));
}

// ── semantic index load ───────────────────────────────────────────────────────

pub fn load_semantic(repo: &RepoInfo) -> Result<SemanticIndex> {
    let sem_dir = semantic_dir(repo);
    let meta_file = meta_path(&sem_dir);

    if !meta_file.exists() {
        bail!(
            "no semantic index found at {}.\n\
             Run `agentgrep index --semantic` to build one.",
            display_path(&sem_dir)
        );
    }

    let meta_bytes = fs::read(&meta_file).context("failed to read meta.json")?;
    let meta: SemanticMeta =
        serde_json::from_slice(&meta_bytes).context("failed to parse meta.json")?;

    // Staleness: fail if both revs are present and differ.
    if let (Some(indexed_rev), Some(repo_rev)) = (&meta.repo_rev, &repo.rev) {
        if indexed_rev != repo_rev {
            bail!(
                "semantic index is stale (indexed at {indexed_rev}, current rev {repo_rev}).\n\
                 Run `agentgrep index --semantic` to refresh it."
            );
        }
    }

    let vectors = read_vectors(&vectors_path(&sem_dir))?;

    if vectors.len() != meta.files.len() {
        bail!(
            "semantic index is corrupt ({} file entries, {} vectors).\n\
             Run `agentgrep index --semantic` to rebuild it.",
            meta.files.len(),
            vectors.len()
        );
    }

    Ok(SemanticIndex { meta, vectors })
}

// ── cosine similarity ─────────────────────────────────────────────────────────

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a < 1e-8 || norm_b < 1e-8 {
        return 0.0;
    }
    (dot / (norm_a * norm_b)).clamp(-1.0, 1.0)
}

// ── find --semantic: expand candidates ───────────────────────────────────────

/// Load model + semantic index, embed the query, run cosine search, and merge
/// semantic candidates with the deterministic candidates already ranked by
/// `rank::rank_with_index`.
///
/// Returns `(merged_candidates, semantic_status_string)`.
pub fn expand_candidates(
    repo: &RepoInfo,
    query: &str,
    mut det_candidates: Vec<FileCandidate>,
) -> Result<(Vec<FileCandidate>, String)> {
    let cache_dir = model_cache_dir()?;
    let model = load_model_for_find(&cache_dir)?;
    let sem_index = load_semantic(repo)?;

    // Embed the query.
    let mut embeddings = model
        .embed(vec![query], None)
        .context("failed to embed query")?;
    let query_vec = embeddings
        .pop()
        .context("no embedding returned for query")?;

    // Brute-force cosine search.
    let mut scored: Vec<(String, f32)> = sem_index
        .meta
        .files
        .iter()
        .zip(sem_index.vectors.iter())
        .filter_map(|(entry, vec)| {
            let sim = cosine_similarity(&query_vec, vec);
            if sim >= COSINE_THRESHOLD {
                Some((entry.path.clone(), sim))
            } else {
                None
            }
        })
        .collect();
    scored.sort_by(|(_, a), (_, b)| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(SEMANTIC_TOP_K);

    if scored.is_empty() {
        return Ok((det_candidates, "active".to_string()));
    }

    // Build path → index map for existing deterministic candidates.
    let mut det_map: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (i, c) in det_candidates.iter().enumerate() {
        det_map.insert(c.path.clone(), i);
    }

    for (path, similarity) in &scored {
        let evidence = Evidence {
            evidence_type: "semantic_match".to_string(),
            detail: format!("cosine {:.3} ({})", similarity, MODEL_NAME),
        };

        if let Some(&idx) = det_map.get(path.as_str()) {
            // Already in deterministic results: annotate + small score boost.
            det_candidates[idx].evidence.push(evidence);
            det_candidates[idx].score += (*similarity * 0.3) as f64;
        } else {
            // Semantic-only candidate.
            let role = crate::index::classify_role(path).to_string();
            let score = (*similarity * 0.8) as f64;
            let confidence = if *similarity >= 0.6 {
                Confidence::Medium
            } else {
                Confidence::Low
            };
            det_candidates.push(FileCandidate {
                path: path.clone(),
                kind: "file".to_string(),
                role,
                score,
                confidence,
                line_ranges: vec![],
                snippets: vec![],
                evidence: vec![evidence],
            });
        }
    }

    // Re-rank merged list; keep within candidate limit.
    det_candidates.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.path.cmp(&b.path))
    });
    det_candidates.truncate(crate::rank::CANDIDATE_LIMIT);

    Ok((det_candidates, "active".to_string()))
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_availability_returns_available() {
        assert_eq!(check_availability(), SemanticState::Available);
    }

    #[test]
    fn require_configured_is_ok() {
        assert!(require_configured("find").is_ok());
        assert!(require_configured("index").is_ok());
    }

    #[test]
    fn cosine_identical() {
        let a = vec![1.0f32, 0.0, 0.0];
        assert!((cosine_similarity(&a, &a) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_orthogonal() {
        let a = vec![1.0f32, 0.0, 0.0];
        let b = vec![0.0f32, 1.0, 0.0];
        assert!(cosine_similarity(&a, &b).abs() < 1e-6);
    }

    #[test]
    fn cosine_zero_vector_returns_zero() {
        let a = vec![1.0f32, 0.0, 0.0];
        let b = vec![0.0f32, 0.0, 0.0];
        assert_eq!(cosine_similarity(&a, &b), 0.0);
    }

    #[test]
    fn cosine_negative_clamp() {
        let a = vec![1.0f32, 0.0];
        let b = vec![-2.0f32, 0.0];
        // Should be exactly -1.0 after clamping.
        assert!((cosine_similarity(&a, &b) - (-1.0)).abs() < 1e-6);
    }

    #[test]
    fn build_file_document_contains_all_parts() {
        let doc = build_file_document(
            "src/foo.rs",
            "source",
            &["Foo".to_string(), "bar".to_string()],
            "pub struct Foo {}",
        );
        assert!(doc.contains("src/foo.rs"), "missing path");
        assert!(doc.contains("source"), "missing role");
        assert!(doc.contains("Foo"), "missing symbol");
        assert!(doc.contains("pub struct Foo {}"), "missing content");
    }

    #[test]
    fn build_file_document_no_symbols() {
        let doc = build_file_document("README.md", "doc", &[], "# Title");
        assert!(!doc.contains("symbols:"), "should not emit symbols line");
        assert!(doc.contains("# Title"), "missing content");
    }

    #[test]
    fn write_and_read_vectors_roundtrip() {
        let vecs = vec![
            vec![1.0f32, 2.0, 3.0],
            vec![4.0f32, 5.0, 6.0],
            vec![-1.0f32, 0.0, 0.5],
        ];
        let dir = temp_dir_for_test();
        let path = dir.join("test.bin");
        write_vectors(&path, &vecs).expect("write failed");
        let got = read_vectors(&path).expect("read failed");
        assert_eq!(vecs.len(), got.len());
        for (expected, actual) in vecs.iter().zip(got.iter()) {
            for (e, a) in expected.iter().zip(actual.iter()) {
                assert!((e - a).abs() < 1e-7, "f32 round-trip mismatch: {e} vs {a}");
            }
        }
        // Clean up.
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_and_read_empty_vectors() {
        let vecs: Vec<Vec<f32>> = vec![];
        let dir = temp_dir_for_test();
        let path = dir.join("empty.bin");
        write_vectors(&path, &vecs).expect("write failed");
        let got = read_vectors(&path).expect("read failed");
        assert!(got.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    fn temp_dir_for_test() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos();
        let dir = std::env::temp_dir().join(format!("agentgrep-sem-test-{nanos}"));
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }
}
