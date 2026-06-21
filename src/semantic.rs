/// Semantic retrieval backend â€” Milestone 8.
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

// â”€â”€ constants â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

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

/// Warn when a repo has this many files to embed (slow on first run).
const FILE_WARN_THRESHOLD: usize = 5_000;

/// Hard limit on files embedded in a single run to bound memory usage.
const FILE_HARD_CAP: usize = 50_000;

// â”€â”€ legacy availability types (kept for tests; no longer called from main) â”€â”€â”€

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

// â”€â”€ metadata types â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

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

// â”€â”€ platform paths â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

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

// â”€â”€ model management â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

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
                "download declined â€” semantic indexing requires the model.\n  \
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

// â”€â”€ document building â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

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

// â”€â”€ binary vector storage â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
//
// Format (little-endian):
//   u32  num_vectors
//   u32  dimensions
//   [num_vectors Ã— dimensions Ã— f32]

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

// â”€â”€ semantic index build â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

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

    // Build a symbol lookup: file_path â†’ Vec<symbol_name>
    let mut symbol_lookup: std::collections::HashMap<&str, Vec<String>> =
        std::collections::HashMap::new();
    for sym in &idx.symbols {
        symbol_lookup
            .entry(sym.file_path.as_str())
            .or_default()
            .push(sym.name.clone());
    }

    if idx.files.len() > FILE_HARD_CAP {
        bail!(
            "repo has {} indexed files, which exceeds the semantic index hard cap of {}.\n\
             Exclude generated or vendor directories (e.g. target/, node_modules/) and rerun.",
            idx.files.len(),
            FILE_HARD_CAP
        );
    }
    if idx.files.len() > FILE_WARN_THRESHOLD {
        eprintln!(
            "Warning: embedding {} files â€” first run may be slow.",
            idx.files.len()
        );
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

// â”€â”€ semantic index load â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

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
        serde_json::from_slice(&meta_bytes).context("failed to parse meta.json â€” index may be corrupt; run `agentgrep index --semantic` to rebuild")?;

    // Schema version check: catch future upgrades.
    if meta.schema_version != SEMANTIC_SCHEMA_VERSION {
        bail!(
            "semantic index uses schema version {} but this binary expects {}.\n\
             Run `agentgrep index --semantic` to rebuild.",
            meta.schema_version,
            SEMANTIC_SCHEMA_VERSION
        );
    }

    // Model compatibility: fail if the index was built with a different model or dimensions.
    if meta.model_name != MODEL_NAME || meta.dimensions != EMBEDDING_DIMS {
        bail!(
            "semantic index was built with {} ({} dims) but current model is {} ({} dims).\n\
             Run `agentgrep index --semantic` to rebuild.",
            meta.model_name,
            meta.dimensions,
            MODEL_NAME,
            EMBEDDING_DIMS
        );
    }

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

// â”€â”€ cosine similarity â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a < 1e-8 || norm_b < 1e-8 {
        return 0.0;
    }
    (dot / (norm_a * norm_b)).clamp(-1.0, 1.0)
}

// â”€â”€ find --semantic: expand candidates â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

/// Returns true when `query` looks like a code identifier rather than natural language.
///
/// Heuristic: no whitespace AND (contains an uppercase letter after the first character,
/// i.e. CamelCase, OR contains an underscore, i.e. snake_case).
///
/// Examples that return true:  `SearchResult`, `SemanticState`, `run_rg`, `expand_candidates`
/// Examples that return false: `where is semantic provider configured`, `searchresult`, `foo`
pub fn is_identifier_like(query: &str) -> bool {
    if query.contains(char::is_whitespace) {
        return false;
    }
    let has_camel = query.char_indices().skip(1).any(|(_, c)| c.is_uppercase());
    let has_snake = query.contains('_');
    has_camel || has_snake
}

/// Returns true when a candidate qualifies as a protected deterministic anchor
/// that semantic scores must not displace.
///
/// A protected anchor is a **source or config** file whose exact query phrase
/// was found in its content (`exact_phrase_match` evidence), and that is not
/// flagged as fixture-like.  Such candidates represent strong, direct lexical
/// evidence that the query targets a specific string (e.g. an error message or
/// API constant) rather than a conceptual topic.
fn is_strong_anchor(candidate: &FileCandidate) -> bool {
    let has_exact = candidate
        .evidence
        .iter()
        .any(|e| e.evidence_type == "exact_phrase_match");
    let is_source_or_config = matches!(candidate.role.as_str(), "source" | "config");
    let not_fixture = !candidate
        .evidence
        .iter()
        .any(|e| e.evidence_type == "fixture_like_match");
    has_exact && is_source_or_config && not_fixture
}

/// Returns true when a candidate is a protected index-definition anchor for
/// the queried symbol.
///
/// A strong symbol anchor is a **source or config** file that carries an
/// `indexed_symbol_definition` evidence entry whose symbol name matches the
/// query (case-insensitive), and that is not flagged as fixture-like.  Such
/// candidates represent the authoritative definition site for a named symbol
/// and must not be displaced by files that only reference, re-export, or are
/// thematically similar to that symbol.
fn is_strong_symbol_anchor(candidate: &FileCandidate, query: &str) -> bool {
    let query_lower = query.to_lowercase();
    let prefix = "defines symbol ";
    let has_exact_sym_def = candidate.evidence.iter().any(|e| {
        e.evidence_type == "indexed_symbol_definition"
            && e.detail
                .strip_prefix(prefix)
                .map(|sym| sym.to_lowercase() == query_lower)
                .unwrap_or(false)
    });
    let is_source_or_config = matches!(candidate.role.as_str(), "source" | "config");
    let not_fixture = !candidate
        .evidence
        .iter()
        .any(|e| e.evidence_type == "fixture_like_match");
    has_exact_sym_def && is_source_or_config && not_fixture
}

/// After semantic re-ranking, prevent semantically-boosted candidates from
/// displacing strong deterministic anchors.
///
/// Two independent guards run in sequence:
///
/// 1. **Exact-phrase guard** â€” if any source/config file contains the verbatim
///    query phrase (`exact_phrase_match`, not fixture-like), every non-phrase-
///    anchor candidate is capped below the best phrase-anchor score.
///
/// 2. **Symbol-definition guard** â€” if any source/config file holds an
///    `indexed_symbol_definition` entry whose symbol name matches the query
///    (not fixture-like), every candidate *without* such a definition is capped
///    below the best symbol-anchor score.  This prevents the printer-subsystem
///    halo effect where many files referencing a symbol get a collective
///    semantic boost that buries the single file that actually defines it.
///
/// Either guard fires only when its class of anchor exists; queries that
/// produce no qualifying anchors (purely conceptual topics with no verbatim
/// phrase and no exact symbol definition) pass through untouched so semantic
/// wins are fully preserved.
fn apply_semantic_anchor_guard(candidates: &mut Vec<FileCandidate>, query: &str) {
    let best_phrase_anchor_score = candidates
        .iter()
        .filter(|c| is_strong_anchor(c))
        .map(|c| c.score)
        .fold(f64::NEG_INFINITY, f64::max);

    let best_symbol_anchor_score = candidates
        .iter()
        .filter(|c| is_strong_symbol_anchor(c, query))
        .map(|c| c.score)
        .fold(f64::NEG_INFINITY, f64::max);

    let mut changed = false;

    if best_phrase_anchor_score.is_finite() {
        for c in candidates.iter_mut() {
            if !is_strong_anchor(c) && c.score >= best_phrase_anchor_score - 1e-9 {
                c.score = (best_phrase_anchor_score - 0.01).max(0.0);
                changed = true;
            }
        }
    }

    if best_symbol_anchor_score.is_finite() {
        for c in candidates.iter_mut() {
            if !is_strong_symbol_anchor(c, query) && c.score >= best_symbol_anchor_score - 1e-9 {
                c.score = (best_symbol_anchor_score - 0.01).max(0.0);
                changed = true;
            }
        }
    }

    if changed {
        candidates.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.path.cmp(&b.path))
        });
    }
}

/// Load model + semantic index, embed the query, run cosine search, and merge
/// semantic candidates with the deterministic candidates already ranked by
/// `rank::rank_with_index`.
///
/// For identifier-like queries (CamelCase / snake_case, no spaces), semantic results
/// only annotate existing deterministic candidates â€” no score boost and no new
/// semantic-only candidates are injected, so deterministic ranking is fully preserved.
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

    let identifier_query = is_identifier_like(query);

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

    // Build path â†’ index map for existing deterministic candidates.
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
            // Already in deterministic results: annotate with evidence.
            // For identifier-like queries skip the score boost so deterministic
            // ranking is not disturbed.
            det_candidates[idx].evidence.push(evidence);
            if !identifier_query {
                det_candidates[idx].score += (*similarity * 0.3) as f64;
            }
        } else if !identifier_query {
            // Semantic-only candidate: not added for identifier queries.
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
                detail_level: crate::types::DetailLevel::Full,
                line_ranges: vec![],
                snippets: vec![],
                evidence: vec![evidence],
                symbols: vec![],
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

    // Identifier queries are annotation-only (no score changes happen above),
    // so the anchor guard is irrelevant and skipped for them.
    if !identifier_query {
        apply_semantic_anchor_guard(&mut det_candidates, query);
    }

    Ok((det_candidates, "active".to_string()))
}

// â”€â”€ semantic status â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

pub struct SemanticStatusReport {
    pub semantic_dir: PathBuf,
    pub semantic_index_exists: bool,
    pub model_cache_dir: PathBuf,
    pub model_cached: bool,
    /// Parsed from meta.json when the index exists; None if missing or unreadable.
    pub meta: Option<SemanticMeta>,
}

pub fn semantic_status(repo: &RepoInfo) -> Result<SemanticStatusReport> {
    let sem_dir = semantic_dir(repo);
    let cache_dir = model_cache_dir()?;
    let semantic_index_exists = meta_path(&sem_dir).exists();
    let model_cached = is_model_cached(&cache_dir);
    let meta = if semantic_index_exists {
        fs::read(meta_path(&sem_dir))
            .ok()
            .and_then(|b| serde_json::from_slice::<SemanticMeta>(&b).ok())
    } else {
        None
    };
    Ok(SemanticStatusReport {
        semantic_dir: sem_dir,
        semantic_index_exists,
        model_cache_dir: cache_dir,
        model_cached,
        meta,
    })
}

pub fn write_semantic_status_report(report: &SemanticStatusReport) {
    println!("Semantic status:");
    let index_state = if report.semantic_index_exists {
        "found"
    } else {
        "not found"
    };
    println!(
        "  Semantic index:  {}  [{}]",
        display_path(&report.semantic_dir),
        index_state
    );
    if let Some(meta) = &report.meta {
        println!(
            "    model:       {} ({} dims)",
            meta.model_name, meta.dimensions
        );
        println!("    files:       {}", meta.files.len());
        if let Some(rev) = &meta.repo_rev {
            println!("    repo_rev:    {}", rev);
        }
        println!("    created_at:  {}", meta.created_at);
    }
    let model_state = if report.model_cached {
        "found"
    } else {
        "not found"
    };
    println!(
        "  Model cache:     {}  [{}]",
        display_path(&report.model_cache_dir),
        model_state
    );
    if !report.model_cached {
        println!(
            "  Tip: run `agentgrep index --semantic` to download the model and build the index."
        );
    }
}

// â”€â”€ semantic clean â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

pub struct SemanticCleanReport {
    pub repo_index_path: Option<PathBuf>,
    pub repo_index_removed: bool,
    pub model_path: Option<PathBuf>,
    pub model_removed: bool,
}

fn remove_dir_if_exists(path: &Path) -> bool {
    if path.exists() {
        fs::remove_dir_all(path).is_ok()
    } else {
        false
    }
}

pub fn clean_repo_index(repo: &RepoInfo) -> Result<SemanticCleanReport> {
    let sem_dir = semantic_dir(repo);
    let removed = remove_dir_if_exists(&sem_dir);
    Ok(SemanticCleanReport {
        repo_index_path: Some(sem_dir),
        repo_index_removed: removed,
        model_path: None,
        model_removed: false,
    })
}

pub fn clean_model_cache() -> Result<SemanticCleanReport> {
    let cache_dir = model_cache_dir()?;
    let removed = remove_dir_if_exists(&cache_dir);
    Ok(SemanticCleanReport {
        repo_index_path: None,
        repo_index_removed: false,
        model_path: Some(cache_dir),
        model_removed: removed,
    })
}

pub fn clean_all(repo: &RepoInfo) -> Result<SemanticCleanReport> {
    let sem_dir = semantic_dir(repo);
    let repo_removed = remove_dir_if_exists(&sem_dir);
    let cache_dir = model_cache_dir()?;
    let model_removed = remove_dir_if_exists(&cache_dir);
    Ok(SemanticCleanReport {
        repo_index_path: Some(sem_dir),
        repo_index_removed: repo_removed,
        model_path: Some(cache_dir),
        model_removed: model_removed,
    })
}

pub fn write_semantic_clean_report(report: &SemanticCleanReport) {
    if let Some(path) = &report.repo_index_path {
        let state = if report.repo_index_removed {
            "removed"
        } else {
            "not present"
        };
        println!("  Semantic index:  {}  [{}]", display_path(path), state);
    }
    if let Some(path) = &report.model_path {
        let state = if report.model_removed {
            "removed"
        } else {
            "not present"
        };
        println!("  Model cache:     {}  [{}]", display_path(path), state);
    }
}

// â”€â”€ tests â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

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
    fn identifier_like_camel_case() {
        assert!(is_identifier_like("SearchResult"));
        assert!(is_identifier_like("SemanticState"));
        assert!(is_identifier_like("FileCandidate"));
    }

    #[test]
    fn identifier_like_snake_case() {
        assert!(is_identifier_like("run_rg"));
        assert!(is_identifier_like("expand_candidates"));
        assert!(is_identifier_like("read_file_preview"));
    }

    #[test]
    fn identifier_like_natural_language_rejected() {
        assert!(!is_identifier_like("where is semantic provider configured"));
        assert!(!is_identifier_like("SearchResult and related"));
        assert!(!is_identifier_like("how does auth work"));
    }

    #[test]
    fn identifier_like_plain_word_rejected() {
        // All lowercase, no separators: not identifier-like.
        assert!(!is_identifier_like("searchresult"));
        assert!(!is_identifier_like("foo"));
        assert!(!is_identifier_like("auth"));
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

    #[test]
    fn semantic_anchor_guard_caps_doc_below_source_anchor() {
        // Scenario: source file has exact_phrase_match; doc file outranked it
        // after semantic boosting (near_phrase only, no exact phrase).
        // The guard must restore the source file to the top position.
        let source = FileCandidate {
            path: "crates/core/search.rs".to_string(),
            kind: "file".to_string(),
            role: "source".to_string(),
            score: 0.75,
            confidence: Confidence::High,
            detail_level: crate::types::DetailLevel::Full,
            line_ranges: vec![],
            snippets: vec![],
            evidence: vec![
                Evidence {
                    evidence_type: "exact_phrase_match".to_string(),
                    detail: "matched exact phrase in lines 310-310".to_string(),
                },
                Evidence {
                    evidence_type: "rg_match".to_string(),
                    detail: "matched on lines 310".to_string(),
                },
                Evidence {
                    evidence_type: "source_role".to_string(),
                    detail: "path suggests source-like file role".to_string(),
                },
            ],
            symbols: vec![],
        };
        let guide = FileCandidate {
            path: "GUIDE.md".to_string(),
            kind: "file".to_string(),
            role: "doc".to_string(),
            score: 0.90,
            confidence: Confidence::Medium,
            detail_level: crate::types::DetailLevel::Full,
            line_ranges: vec![],
            snippets: vec![],
            evidence: vec![
                Evidence {
                    evidence_type: "near_phrase_match".to_string(),
                    detail: "2 query terms clustered in lines 148-149".to_string(),
                },
                Evidence {
                    evidence_type: "semantic_match".to_string(),
                    detail: "cosine 0.850 (BAAI/bge-small-en-v1.5)".to_string(),
                },
            ],
            symbols: vec![],
        };

        let mut candidates = vec![guide, source];
        apply_semantic_anchor_guard(&mut candidates, "query");

        assert_eq!(
            candidates[0].path, "crates/core/search.rs",
            "source with exact_phrase_match must rank first"
        );
        assert_eq!(candidates[1].path, "GUIDE.md");
        assert!(
            candidates[0].score > candidates[1].score,
            "source score must exceed guide score after capping"
        );
    }

    #[test]
    fn semantic_anchor_guard_no_op_without_exact_phrase_anchor() {
        // Conceptual query: no source/config file has exact_phrase_match.
        // Semantic win for a doc file must be preserved unchanged.
        let doc = FileCandidate {
            path: "docs/architecture.md".to_string(),
            kind: "file".to_string(),
            role: "doc".to_string(),
            score: 0.90,
            confidence: Confidence::Medium,
            detail_level: crate::types::DetailLevel::Full,
            line_ranges: vec![],
            snippets: vec![],
            evidence: vec![Evidence {
                evidence_type: "semantic_match".to_string(),
                detail: "cosine 0.850 (BAAI/bge-small-en-v1.5)".to_string(),
            }],
            symbols: vec![],
        };
        let source = FileCandidate {
            path: "src/rank.rs".to_string(),
            kind: "file".to_string(),
            role: "source".to_string(),
            score: 0.75,
            confidence: Confidence::Medium,
            detail_level: crate::types::DetailLevel::Full,
            line_ranges: vec![],
            snippets: vec![],
            evidence: vec![
                Evidence {
                    evidence_type: "near_phrase_match".to_string(),
                    detail: "terms clustered near line 42".to_string(),
                },
                Evidence {
                    evidence_type: "rg_match".to_string(),
                    detail: "matched on lines 42".to_string(),
                },
            ],
            symbols: vec![],
        };

        let mut candidates = vec![doc, source];
        apply_semantic_anchor_guard(&mut candidates, "conceptual query");

        assert_eq!(
            candidates[0].path, "docs/architecture.md",
            "doc semantic win must be preserved when no exact phrase anchor exists"
        );
        assert_eq!(candidates[1].path, "src/rank.rs");
        assert!(
            (candidates[0].score - 0.90).abs() < 1e-9,
            "doc score must be unchanged"
        );
    }

    #[test]
    fn semantic_anchor_guard_caps_doc_that_ties_anchor_score() {
        // Reproduces the ripgrep-err-002 failure mode: the doc file ties the
        // anchor's score exactly (both capped to 1.0 by the scoring pipeline),
        // so a strict `>` comparison would leave it uncapped.
        let source = FileCandidate {
            path: "crates/core/search.rs".to_string(),
            kind: "file".to_string(),
            role: "source".to_string(),
            score: 1.0,
            confidence: Confidence::High,
            detail_level: crate::types::DetailLevel::Full,
            line_ranges: vec![],
            snippets: vec![],
            evidence: vec![
                Evidence {
                    evidence_type: "exact_phrase_match".to_string(),
                    detail: "matched exact phrase in lines 310-310".to_string(),
                },
                Evidence {
                    evidence_type: "source_role".to_string(),
                    detail: "path suggests source-like file role".to_string(),
                },
            ],
            symbols: vec![],
        };
        let guide = FileCandidate {
            path: "GUIDE.md".to_string(),
            kind: "file".to_string(),
            role: "doc".to_string(),
            score: 1.0,
            confidence: Confidence::Medium,
            detail_level: crate::types::DetailLevel::Full,
            line_ranges: vec![],
            snippets: vec![],
            evidence: vec![
                Evidence {
                    evidence_type: "near_phrase_match".to_string(),
                    detail: "2 query terms clustered in lines 148-149".to_string(),
                },
                Evidence {
                    evidence_type: "semantic_match".to_string(),
                    detail: "cosine 0.850 (BAAI/bge-small-en-v1.5)".to_string(),
                },
            ],
            symbols: vec![],
        };

        // GUIDE.md sorts first alphabetically at equal scores; guard must fix this.
        let mut candidates = vec![guide, source];
        apply_semantic_anchor_guard(&mut candidates, "query");

        assert_eq!(
            candidates[0].path, "crates/core/search.rs",
            "source must rank first even when scores tie"
        );
        assert!(
            candidates[0].score > candidates[1].score,
            "guide score must be capped below anchor score"
        );
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




