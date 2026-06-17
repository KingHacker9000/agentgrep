use anyhow::{Context, Result};
use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::collections::HashSet;
use std::fs;
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::parser::extracted::{dedup_edges, dedup_symbols, ImportBinding};
use crate::parser::language::{parent_dir, RepoLookup};
use crate::repo::{display_path, RepoInfo};
use crate::text::shorten_snippet;
use crate::types::{IndexedSymbol, SymbolKind, Visibility};
use serde_json::Value;

pub const INDEX_SCHEMA_VERSION: u32 = 6;
pub const HASH_LIMIT_BYTES: u64 = 1024 * 256;
pub const MAX_LEX_TERMS: usize = 300;
const LEX_READ_SIZE_LIMIT: u64 = HASH_LIMIT_BYTES;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoIndex {
    pub schema_version: u32,
    pub repo_root: String,
    pub repo_rev: Option<String>,
    pub indexed_at_unix: u64,
    pub files: Vec<IndexedFile>,
    #[serde(default)]
    pub symbols: Vec<IndexedSymbol>,
    #[serde(default)]
    pub symbol_references: Vec<IndexedSymbolReference>,
    pub edges: Vec<IndexedEdge>,
    pub stats: IndexStats,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FileLexStats {
    pub doc_length: u32,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub term_frequencies: BTreeMap<String, u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IndexedFile {
    pub path: String,
    pub role: FileRole,
    pub size_bytes: Option<u64>,
    pub modified_unix: Option<u64>,
    pub content_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lex_stats: Option<FileLexStats>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexedSymbolReference {
    pub from_file: String,
    pub symbol_name: String,
    pub target_file: Option<String>,
    pub target_line: Option<usize>,
    pub line_number: usize,
    pub confidence: EdgeConfidence,
    pub reason: String,
    #[serde(default)]
    pub context: ReferenceContext,
    #[serde(default)]
    pub additional_count: usize,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum ReferenceContext {
    Production,
    Test,
    Fixture,
    Unknown,
}

impl Default for ReferenceContext {
    fn default() -> Self {
        ReferenceContext::Production
    }
}

impl std::fmt::Display for ReferenceContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            ReferenceContext::Production => "production",
            ReferenceContext::Test => "test",
            ReferenceContext::Fixture => "fixture",
            ReferenceContext::Unknown => "unknown",
        };
        write!(f, "{value}")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexedEdge {
    pub edge_type: String,
    pub from: String,
    pub to: String,
    pub confidence: EdgeConfidence,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IndexStats {
    pub file_count: usize,
    pub role_counts: BTreeMap<FileRole, usize>,
    #[serde(default)]
    pub symbol_count: usize,
    #[serde(default)]
    pub symbol_kind_counts: BTreeMap<SymbolKind, usize>,
    #[serde(default)]
    pub symbol_reference_count: usize,
    pub connection_count: usize,
    #[serde(default)]
    pub lex_file_count: usize,
    #[serde(default)]
    pub avg_doc_length: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Default)]
#[serde(rename_all = "lowercase")]
pub enum FileRole {
    Source,
    Test,
    Doc,
    Config,
    Lockfile,
    Generated,
    #[default]
    Other,
}

impl std::fmt::Display for FileRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            FileRole::Source => "source",
            FileRole::Test => "test",
            FileRole::Doc => "doc",
            FileRole::Config => "config",
            FileRole::Lockfile => "lockfile",
            FileRole::Generated => "generated",
            FileRole::Other => "other",
        };
        write!(f, "{value}")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EdgeConfidence {
    Extracted,
    Inferred,
    Ambiguous,
}

impl std::fmt::Display for EdgeConfidence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            EdgeConfidence::Extracted => "extracted",
            EdgeConfidence::Inferred => "inferred",
            EdgeConfidence::Ambiguous => "ambiguous",
        };
        write!(f, "{value}")
    }
}

#[derive(Debug)]
pub struct IndexBuildReport {
    pub index_path: PathBuf,
    pub repo_rev: Option<String>,
    pub file_count: usize,
    pub role_counts: BTreeMap<FileRole, usize>,
    pub symbol_count: usize,
    pub symbol_kind_counts: BTreeMap<SymbolKind, usize>,
    pub symbol_reference_count: usize,
    pub connection_count: usize,
    pub lex_file_count: usize,
    pub avg_doc_length: f64,
}

#[derive(Debug)]
pub struct IndexStatusReport {
    pub index_path: PathBuf,
    pub state: IndexState,
    pub file_count: usize,
    pub role_counts: BTreeMap<FileRole, usize>,
    pub symbol_count: usize,
    pub symbol_kind_counts: BTreeMap<SymbolKind, usize>,
    pub symbol_reference_count: usize,
    pub connection_count: usize,
    pub repo_rev: Option<String>,
    pub indexed_rev: Option<String>,
    pub lex_file_count: usize,
    pub avg_doc_length: f64,
}

#[derive(Debug, Clone)]
pub struct LoadedIndex {
    pub index_path: PathBuf,
    pub state: IndexState,
    pub index: Option<RepoIndex>,
}

#[derive(Debug)]
pub struct IndexClearReport {
    pub index_path: PathBuf,
    pub existed: bool,
    pub cleared: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexState {
    Missing,
    Fresh,
    Stale,
    Unverifiable,
}

impl std::fmt::Display for IndexState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            IndexState::Missing => "missing",
            IndexState::Fresh => "fresh",
            IndexState::Stale => "stale",
            IndexState::Unverifiable => "unverifiable",
        };
        write!(f, "{value}")
    }
}

pub fn index_path(repo: &RepoInfo) -> PathBuf {
    match &repo.git_dir {
        Some(git_dir) => git_dir.join("agentgrep").join("index.json"),
        None => repo.root.join(".agentgrep").join("index.json"),
    }
}

pub fn build(repo: &RepoInfo) -> Result<IndexBuildReport> {
    let index_path = index_path(repo);
    let index = build_index(repo)?;
    write_index_file(&index_path, &index)?;

    Ok(IndexBuildReport {
        index_path,
        repo_rev: index.repo_rev.clone(),
        file_count: index.stats.file_count,
        role_counts: index.stats.role_counts.clone(),
        symbol_count: index.stats.symbol_count,
        symbol_kind_counts: index.stats.symbol_kind_counts.clone(),
        symbol_reference_count: index.stats.symbol_reference_count,
        connection_count: index.stats.connection_count,
        lex_file_count: index.stats.lex_file_count,
        avg_doc_length: index.stats.avg_doc_length,
    })
}

pub fn status(repo: &RepoInfo) -> Result<IndexStatusReport> {
    let loaded = load(repo)?;

    if let Some(index) = loaded.index {
        Ok(IndexStatusReport {
            index_path: loaded.index_path,
            state: loaded.state,
            file_count: index.stats.file_count,
            role_counts: index.stats.role_counts,
            symbol_count: index.stats.symbol_count,
            symbol_kind_counts: index.stats.symbol_kind_counts,
            symbol_reference_count: index.stats.symbol_reference_count,
            connection_count: index.stats.connection_count,
            repo_rev: repo.rev.clone(),
            indexed_rev: index.repo_rev,
            lex_file_count: index.stats.lex_file_count,
            avg_doc_length: index.stats.avg_doc_length,
        })
    } else {
        Ok(IndexStatusReport {
            index_path: loaded.index_path,
            state: loaded.state,
            file_count: 0,
            role_counts: BTreeMap::new(),
            symbol_count: 0,
            symbol_kind_counts: BTreeMap::new(),
            symbol_reference_count: 0,
            connection_count: 0,
            repo_rev: repo.rev.clone(),
            indexed_rev: None,
            lex_file_count: 0,
            avg_doc_length: 0.0,
        })
    }
}

pub fn clear(repo: &RepoInfo) -> Result<IndexClearReport> {
    let index_path = index_path(repo);
    let existed = index_path.exists();
    if existed {
        fs::remove_file(&index_path).with_context(|| {
            format!(
                "failed to remove index file at {}",
                display_path(&index_path)
            )
        })?;
        remove_empty_parent(
            &index_path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_default(),
        )?;
    }

    Ok(IndexClearReport {
        index_path,
        existed,
        cleared: existed,
    })
}

pub fn load(repo: &RepoInfo) -> Result<LoadedIndex> {
    let index_path = index_path(repo);
    let index = read_index_file(&index_path)?;
    let state = determine_state(repo, index.as_ref());

    Ok(LoadedIndex {
        index_path,
        state,
        index,
    })
}

pub fn write_build_report(report: &IndexBuildReport) -> Result<()> {
    println!("Index written:");
    println!("- files indexed: {}", report.file_count);
    println!(
        "- roles counted: {}",
        format_role_counts(&report.role_counts)
    );
    println!("- symbols indexed: {}", report.symbol_count);
    println!(
        "- symbol kinds: {}",
        format_symbol_kind_counts(&report.symbol_kind_counts)
    );
    println!(
        "- symbol references indexed: {}",
        report.symbol_reference_count
    );
    println!("- connections counted: {}", report.connection_count);
    println!(
        "- lexical stats: {} files, avg {:.0} tokens/file",
        report.lex_file_count, report.avg_doc_length
    );
    println!("- index path: {}", display_path(&report.index_path));
    println!(
        "- repo rev: {}",
        report.repo_rev.as_deref().unwrap_or("not available")
    );
    Ok(())
}

pub fn write_status_report(report: &IndexStatusReport) -> Result<()> {
    println!("Index status: {}", report.state);
    println!("- index path: {}", display_path(&report.index_path));
    println!("- files indexed: {}", report.file_count);
    println!(
        "- roles counted: {}",
        format_role_counts(&report.role_counts)
    );
    println!("- symbols indexed: {}", report.symbol_count);
    println!(
        "- symbol kinds: {}",
        format_symbol_kind_counts(&report.symbol_kind_counts)
    );
    println!(
        "- symbol references indexed: {}",
        report.symbol_reference_count
    );
    println!("- connections counted: {}", report.connection_count);
    println!(
        "- lexical stats: {} files, avg {:.0} tokens/file",
        report.lex_file_count, report.avg_doc_length
    );
    if let Some(repo_rev) = &report.repo_rev {
        println!("- repo rev: {}", repo_rev);
    }
    if let Some(indexed_rev) = &report.indexed_rev {
        println!("- indexed rev: {}", indexed_rev);
    }
    if report.state == IndexState::Unverifiable && report.repo_rev.is_none() {
        println!("- note: unverifiable because repo revision is unavailable");
    }
    Ok(())
}

pub fn write_clear_report(report: &IndexClearReport) -> Result<()> {
    if report.cleared {
        println!("Cleared index: {}", display_path(&report.index_path));
    } else {
        println!("No index to clear: {}", display_path(&report.index_path));
    }
    if !report.existed {
        println!("- index file was already missing");
    }
    Ok(())
}

pub fn classify_role(path: &str) -> FileRole {
    let lower = path.to_lowercase();
    if is_generated_path(&lower) {
        return FileRole::Generated;
    }
    if is_lockfile(&lower) {
        return FileRole::Lockfile;
    }
    if is_test_path(&lower) {
        return FileRole::Test;
    }
    if is_doc_path(&lower) {
        return FileRole::Doc;
    }
    if is_config_path(&lower) {
        return FileRole::Config;
    }
    if is_source_path(&lower) {
        return FileRole::Source;
    }
    FileRole::Other
}

pub fn maybe_same_area_key(path: &str, role: &FileRole) -> Option<String> {
    if !matches!(role, FileRole::Source) {
        return None;
    }

    let segments = split_path(path);
    if segments.is_empty() {
        return None;
    }

    if segments[0] == "src"
        || segments[0] == "app"
        || segments[0] == "lib"
        || segments[0] == "services"
    {
        if segments.len() >= 2 && !segments[1].contains('.') {
            return Some(format!("{}/{}", segments[0], segments[1]));
        }
        return Some(segments[0].to_string());
    }

    if segments[0] == "packages" || segments[0] == "modules" || segments[0] == "apps" {
        if segments.len() >= 2 {
            return Some(format!("{}/{}", segments[0], segments[1]));
        }
    }

    Some(segments[0].to_string())
}

pub fn likely_test_targets(
    test_path: &str,
    source_paths: &[String],
) -> Vec<(String, EdgeConfidence, String)> {
    let test_stem = test_stem(test_path);
    let test_tokens = path_tokens(&test_stem);
    let mut scored = Vec::new();

    for source_path in source_paths {
        let source_stem = file_stem(source_path);
        let source_tokens = path_tokens(&source_stem);
        let exact = test_stem == source_stem;
        let token_overlap = shared_token_count(&test_tokens, &source_tokens);
        if exact || token_overlap > 0 {
            let confidence = if exact {
                EdgeConfidence::Extracted
            } else if token_overlap >= 2 {
                EdgeConfidence::Inferred
            } else {
                EdgeConfidence::Ambiguous
            };
            let reason = if exact {
                "filename stem matches".to_string()
            } else {
                format!("shared stem tokens: {}", token_overlap)
            };
            scored.push((source_path.clone(), confidence, reason));
        }
    }

    scored.sort_by(|left, right| left.0.cmp(&right.0));
    scored.truncate(3);
    scored
}

fn compute_file_lex_stats(text: &str) -> FileLexStats {
    let tokens = crate::text::tokenize_lexical(text);
    let doc_length = tokens.len() as u32;

    let mut raw_freq: HashMap<String, u32> = HashMap::new();
    for token in tokens {
        *raw_freq.entry(token).or_insert(0) += 1;
    }

    let mut pairs: Vec<(String, u32)> = raw_freq.into_iter().collect();
    pairs.sort_by(|(a_term, a_freq), (b_term, b_freq)| {
        b_freq.cmp(a_freq).then_with(|| a_term.cmp(b_term))
    });
    pairs.truncate(MAX_LEX_TERMS);

    let term_frequencies: BTreeMap<String, u32> = pairs.into_iter().collect();

    FileLexStats {
        doc_length,
        term_frequencies,
    }
}

fn build_index(repo: &RepoInfo) -> Result<RepoIndex> {
    let mut files = Vec::new();
    let mut source_texts = BTreeMap::new();
    let mut lex_texts = BTreeMap::new();
    collect_files(&repo.root, &mut files, &mut source_texts, &mut lex_texts)?;
    files.sort_by(|left, right| left.path.cmp(&right.path));

    let source_paths: Vec<String> = files
        .iter()
        .filter(|file| matches!(file.role, FileRole::Source))
        .map(|file| file.path.clone())
        .collect();

    let parsed_facts = crate::parser::extract_repo_facts(&files, &source_texts);
    let parsed_facts =
        crate::parser::combine_with_rust_fallback(parsed_facts, &files, &source_texts);
    let mut symbols = parsed_facts.symbols;
    let mut edges = parsed_facts.edges;
    let import_bindings = parsed_facts.symbol_references;
    dedup_symbols(&mut symbols);
    dedup_edges(&mut edges);
    let mut symbol_references = build_rust_symbol_references(&files, &source_texts, &symbols);
    symbol_references.extend(build_import_binding_symbol_references(
        &import_bindings,
        &symbols,
    ));
    dedup_symbol_references(&mut symbol_references);
    let symbol_reference_count = symbol_references.len();
    edges.extend(build_same_area_edges(&files));
    edges.extend(build_test_edges(&files, &source_paths));
    edges.extend(build_config_edges(&repo.root, &files, &source_paths));
    dedup_edges(&mut edges);
    let connection_count = edges.len();

    let mut lex_file_count = 0usize;
    let mut total_doc_length = 0u64;
    for file in &mut files {
        if let Some(text) = lex_texts.get(&file.path) {
            let stats = compute_file_lex_stats(text);
            total_doc_length += stats.doc_length as u64;
            lex_file_count += 1;
            file.lex_stats = Some(stats);
        }
    }
    let avg_doc_length = if lex_file_count > 0 {
        total_doc_length as f64 / lex_file_count as f64
    } else {
        0.0
    };

    let file_count = files.len();
    let role_counts = count_roles(&files);
    let symbol_count = symbols.len();
    let symbol_kind_counts = count_symbol_kinds(&symbols);
    let indexed_at_unix = unix_now();

    Ok(RepoIndex {
        schema_version: INDEX_SCHEMA_VERSION,
        repo_root: display_path(&repo.root),
        repo_rev: repo.rev.clone(),
        indexed_at_unix,
        files,
        symbols,
        symbol_references,
        edges,
        stats: IndexStats {
            file_count,
            role_counts,
            symbol_count,
            symbol_kind_counts,
            symbol_reference_count,
            connection_count,
            lex_file_count,
            avg_doc_length,
        },
    })
}

fn build_same_area_edges(files: &[IndexedFile]) -> Vec<IndexedEdge> {
    let mut grouped: BTreeMap<String, Vec<&IndexedFile>> = BTreeMap::new();
    for file in files {
        if let Some(key) = maybe_same_area_key(&file.path, &file.role) {
            grouped.entry(key).or_default().push(file);
        }
    }

    let mut edges = Vec::new();
    for (area, group) in grouped {
        if group.len() < 2 {
            continue;
        }
        for i in 0..group.len() {
            for j in (i + 1)..group.len() {
                let from = &group[i].path;
                let to = &group[j].path;
                edges.push(IndexedEdge {
                    edge_type: "same_area".to_string(),
                    from: from.clone(),
                    to: to.clone(),
                    confidence: EdgeConfidence::Extracted,
                    reason: format!("shared source area {area}"),
                });
            }
        }
    }
    edges
}

fn build_test_edges(files: &[IndexedFile], source_paths: &[String]) -> Vec<IndexedEdge> {
    let mut edges = Vec::new();
    for file in files
        .iter()
        .filter(|file| matches!(file.role, FileRole::Test))
    {
        for (target, confidence, reason) in likely_test_targets(&file.path, source_paths) {
            edges.push(IndexedEdge {
                edge_type: "likely_test_for".to_string(),
                from: file.path.clone(),
                to: target,
                confidence,
                reason,
            });
        }
    }
    edges
}

fn build_config_edges(
    repo_root: &Path,
    files: &[IndexedFile],
    source_paths: &[String],
) -> Vec<IndexedEdge> {
    let mut edges = Vec::new();
    let source_roots = choose_source_roots(source_paths);
    let lookup = RepoLookup::new(files);

    for file in files
        .iter()
        .filter(|file| matches!(file.role, FileRole::Config | FileRole::Lockfile))
    {
        if let Some(target) = source_roots.first() {
            edges.push(IndexedEdge {
                edge_type: "configures".to_string(),
                from: file.path.clone(),
                to: target.clone(),
                confidence: EdgeConfidence::Inferred,
                reason: "manifest or config points at source root".to_string(),
            });
        }

        if is_browser_extension_manifest(&file.path) {
            let manifest_path = repo_root.join(&file.path);
            if let Ok(text) = fs::read_to_string(&manifest_path) {
                edges.extend(build_manifest_edges(&file.path, &text, &lookup));
            }
        }
    }

    edges
}

fn build_manifest_edges(file_path: &str, source: &str, lookup: &RepoLookup) -> Vec<IndexedEdge> {
    let Ok(value) = serde_json::from_str::<Value>(source) else {
        return Vec::new();
    };

    let mut edges = Vec::new();
    let base_dir = parent_dir(file_path);

    if let Some(background) = value.get("background").and_then(|item| item.as_object()) {
        if let Some(service_worker) = background
            .get("service_worker")
            .and_then(|value| value.as_str())
        {
            push_manifest_reference(
                &mut edges,
                file_path,
                &base_dir,
                service_worker,
                "background.service_worker",
                lookup,
            );
        }

        if let Some(scripts) = background.get("scripts").and_then(|value| value.as_array()) {
            for script in scripts.iter().filter_map(|value| value.as_str()) {
                push_manifest_reference(
                    &mut edges,
                    file_path,
                    &base_dir,
                    script,
                    "background.scripts",
                    lookup,
                );
            }
        }
    }

    if let Some(content_scripts) = value
        .get("content_scripts")
        .and_then(|value| value.as_array())
    {
        for entry in content_scripts.iter().filter_map(|value| value.as_object()) {
            if let Some(js_files) = entry.get("js").and_then(|value| value.as_array()) {
                for script in js_files.iter().filter_map(|value| value.as_str()) {
                    push_manifest_reference(
                        &mut edges,
                        file_path,
                        &base_dir,
                        script,
                        "content_scripts.js",
                        lookup,
                    );
                }
            }

            if let Some(css_files) = entry.get("css").and_then(|value| value.as_array()) {
                for stylesheet in css_files.iter().filter_map(|value| value.as_str()) {
                    push_manifest_reference(
                        &mut edges,
                        file_path,
                        &base_dir,
                        stylesheet,
                        "content_scripts.css",
                        lookup,
                    );
                }
            }
        }
    }

    edges
}

fn push_manifest_reference(
    edges: &mut Vec<IndexedEdge>,
    file_path: &str,
    base_dir: &str,
    raw_path: &str,
    field_name: &str,
    lookup: &RepoLookup,
) {
    if let Some(target) = resolve_manifest_target(base_dir, raw_path, lookup) {
        edges.push(IndexedEdge {
            edge_type: "references".to_string(),
            from: file_path.to_string(),
            to: target.clone(),
            confidence: EdgeConfidence::Extracted,
            reason: format!(
                "manifest {field_name} references {}",
                display_path(std::path::Path::new(&target))
            ),
        });
    }
}

fn resolve_manifest_target(base_dir: &str, raw_path: &str, lookup: &RepoLookup) -> Option<String> {
    let normalized = raw_path.trim().trim_matches('"').trim_matches('\'');
    if normalized.is_empty() {
        return None;
    }

    let mut candidates = Vec::new();
    let joined = if normalized.starts_with("./") || normalized.starts_with("../") {
        let joined = std::path::Path::new(base_dir).join(normalized);
        display_path(&joined)
    } else if normalized.starts_with('/') {
        normalized.trim_start_matches('/').to_string()
    } else if base_dir.is_empty() {
        normalized.to_string()
    } else {
        format!("{base_dir}/{normalized}")
    };

    push_path_variants(&mut candidates, &joined);
    if joined != normalized {
        push_path_variants(&mut candidates, normalized);
    }

    if let Some(target) = lookup.resolve_candidates(candidates.clone()) {
        return Some(target);
    }

    resolve_by_stem(lookup, &candidates)
}

fn push_path_variants(candidates: &mut Vec<String>, path: &str) {
    let normalized = path.replace('\\', "/");
    let stem = normalized
        .rsplit_once('.')
        .map(|(head, _)| head)
        .unwrap_or(normalized.as_str());

    candidates.push(normalized.clone());
    if stem != normalized {
        for ext in [".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs"] {
            candidates.push(format!("{stem}{ext}"));
        }
    } else {
        for ext in [".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs"] {
            candidates.push(format!("{normalized}{ext}"));
        }
        for ext in [".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs"] {
            candidates.push(format!("{normalized}/index{ext}"));
        }
    }
}

fn resolve_by_stem(lookup: &RepoLookup, candidates: &[String]) -> Option<String> {
    let stems = candidates
        .iter()
        .map(|candidate| {
            candidate
                .rsplit_once('.')
                .map(|(head, _)| head.to_string())
                .unwrap_or_else(|| candidate.clone())
        })
        .collect::<Vec<_>>();

    for stem in stems {
        for ext in [
            ".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".css", ".html",
        ] {
            let candidate = format!("{stem}{ext}");
            if let Some(target) = lookup.resolve_candidates([candidate.clone()]) {
                return Some(target);
            }
        }
        for ext in [".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs"] {
            let candidate = format!("{stem}/index{ext}");
            if let Some(target) = lookup.resolve_candidates([candidate.clone()]) {
                return Some(target);
            }
        }
    }

    None
}

pub(crate) fn build_rust_symbols(
    files: &[IndexedFile],
    source_texts: &BTreeMap<String, String>,
) -> Vec<IndexedSymbol> {
    let mut symbols = Vec::new();

    for file in files
        .iter()
        .filter(|file| matches!(file.role, FileRole::Source) && file.path.ends_with(".rs"))
    {
        let Some(text) = source_texts.get(&file.path) else {
            continue;
        };

        for (line_index, line) in text.lines().enumerate() {
            if let Some(symbol) = rust_symbol_from_line(&file.path, line_index + 1, line) {
                symbols.push(symbol);
            }
        }
    }

    symbols
}

fn count_symbol_kinds(symbols: &[IndexedSymbol]) -> BTreeMap<SymbolKind, usize> {
    let mut counts = BTreeMap::new();
    for symbol in symbols {
        *counts.entry(symbol.kind.clone()).or_insert(0) += 1;
    }
    counts
}

fn build_import_binding_symbol_references(
    bindings: &[ImportBinding],
    symbols: &[IndexedSymbol],
) -> Vec<IndexedSymbolReference> {
    let mut definition_lines: BTreeMap<(String, String), usize> = BTreeMap::new();
    for symbol in symbols {
        definition_lines
            .entry((symbol.file_path.clone(), symbol.name.clone()))
            .and_modify(|line| {
                if symbol.line_number < *line {
                    *line = symbol.line_number;
                }
            })
            .or_insert(symbol.line_number);
    }

    let mut references = Vec::new();
    for binding in bindings {
        let Some(target_file) = binding.target_file.as_ref() else {
            continue;
        };
        let Some(target_line) = definition_lines
            .get(&(target_file.clone(), binding.symbol_name.clone()))
            .copied()
        else {
            continue;
        };
        references.push(binding.clone().into_reference(Some(target_line)));
    }

    references.sort_by(|left, right| {
        left.from_file
            .cmp(&right.from_file)
            .then_with(|| left.line_number.cmp(&right.line_number))
            .then_with(|| left.symbol_name.cmp(&right.symbol_name))
            .then_with(|| left.target_file.cmp(&right.target_file))
            .then_with(|| left.target_line.cmp(&right.target_line))
            .then_with(|| left.reason.cmp(&right.reason))
    });

    references
}

fn dedup_symbol_references(references: &mut Vec<IndexedSymbolReference>) {
    let mut seen = BTreeSet::new();
    references.retain(|reference| {
        seen.insert((
            reference.from_file.clone(),
            reference.symbol_name.clone(),
            reference.target_file.clone(),
            reference.target_line,
            reference.line_number,
            reference.confidence.clone(),
            reference.reason.clone(),
            reference.context,
            reference.additional_count,
        ))
    });
}

fn build_rust_symbol_references(
    files: &[IndexedFile],
    source_texts: &BTreeMap<String, String>,
    symbols: &[IndexedSymbol],
) -> Vec<IndexedSymbolReference> {
    let symbol_lookup = build_referenceable_symbol_lookup(symbols);
    let referenceable_names = build_referenceable_symbol_names(&symbol_lookup);
    let definition_lines = build_symbol_definition_lines(symbols);
    let mut grouped = BTreeMap::new();

    for file in files
        .iter()
        .filter(|file| matches!(file.role, FileRole::Source) && file.path.ends_with(".rs"))
    {
        let Some(text) = source_texts.get(&file.path) else {
            continue;
        };
        let defined_lines = definition_lines.get(&file.path);
        let mut in_test_section = matches!(file.role, FileRole::Test) || is_test_path(&file.path);
        let mut current_function_context = if in_test_section {
            ReferenceContext::Unknown
        } else {
            ReferenceContext::Production
        };
        let mut pending_test_attribute = false;

        for (line_index, line) in text.lines().enumerate() {
            let line_number = line_index + 1;
            let stripped = strip_line_comment(line).trim();
            if stripped.is_empty() {
                continue;
            }

            if is_test_section_line(stripped) {
                in_test_section = true;
                current_function_context = ReferenceContext::Unknown;
                pending_test_attribute = false;
                continue;
            }
            if stripped.starts_with("#[test]") {
                pending_test_attribute = true;
                continue;
            }

            if defined_lines
                .map(|lines| lines.contains(&line_number))
                .unwrap_or(false)
            {
                continue;
            }

            let line_context = classify_rust_reference_context(
                file,
                line_number,
                stripped,
                current_function_context,
                in_test_section,
            );

            if let Some(use_body) = parse_rust_use_statement(stripped) {
                for path in expand_rust_use_paths(use_body) {
                    if let Some(name) = path.last() {
                        if let Some(reference) = build_symbol_reference_from_name(
                            file,
                            line_number,
                            name,
                            &symbol_lookup,
                            line_context,
                            "use statement reference",
                            EdgeConfidence::Extracted,
                        ) {
                            insert_grouped_reference(&mut grouped, reference);
                        }
                    }
                }
                if let Some(name) = rust_function_name_from_line(stripped) {
                    current_function_context = if pending_test_attribute {
                        pending_test_attribute = false;
                        ReferenceContext::Test
                    } else if in_test_section {
                        classify_test_function_name(&name)
                    } else {
                        ReferenceContext::Production
                    };
                }
                continue;
            }

            let quoted = stripped.contains('"') || stripped.contains('\'');
            for name in &referenceable_names {
                if !contains_identifier_reference(stripped, name) {
                    continue;
                }

                if quoted && !is_strong_test_reference_line(stripped) {
                    continue;
                }
                if in_test_section && !is_strong_test_reference_line(stripped) {
                    continue;
                }

                if let Some(reference) = build_symbol_reference_from_name(
                    file,
                    line_number,
                    name,
                    &symbol_lookup,
                    line_context,
                    "qualified or token reference",
                    EdgeConfidence::Inferred,
                ) {
                    insert_grouped_reference(&mut grouped, reference);
                }
            }

            if let Some(name) = rust_function_name_from_line(stripped) {
                current_function_context = if pending_test_attribute {
                    pending_test_attribute = false;
                    ReferenceContext::Test
                } else if in_test_section {
                    classify_test_function_name(&name)
                } else {
                    ReferenceContext::Production
                };
            }
        }
    }

    let mut references = grouped.into_values().collect::<Vec<_>>();
    references.sort_by(|left, right| {
        reference_context_priority(left.context)
            .cmp(&reference_context_priority(right.context))
            .then_with(|| {
                reference_confidence_priority(&left.confidence)
                    .cmp(&reference_confidence_priority(&right.confidence))
            })
            .then_with(|| left.from_file.cmp(&right.from_file))
            .then_with(|| left.line_number.cmp(&right.line_number))
            .then_with(|| left.symbol_name.cmp(&right.symbol_name))
    });

    references
}

fn build_referenceable_symbol_lookup<'a>(
    symbols: &'a [IndexedSymbol],
) -> BTreeMap<String, Vec<&'a IndexedSymbol>> {
    let mut lookup: BTreeMap<String, Vec<&IndexedSymbol>> = BTreeMap::new();
    for symbol in symbols
        .iter()
        .filter(|symbol| is_referenceable_symbol_kind(&symbol.kind))
    {
        lookup.entry(symbol.name.clone()).or_default().push(symbol);
    }
    lookup
}

fn build_referenceable_symbol_names(
    symbol_lookup: &BTreeMap<String, Vec<&IndexedSymbol>>,
) -> Vec<String> {
    let mut names = symbol_lookup.keys().cloned().collect::<Vec<_>>();
    names.sort_by(|left, right| right.len().cmp(&left.len()).then_with(|| left.cmp(right)));
    names
}

fn build_symbol_definition_lines(symbols: &[IndexedSymbol]) -> BTreeMap<String, HashSet<usize>> {
    let mut lines = BTreeMap::new();
    for symbol in symbols {
        lines
            .entry(symbol.file_path.clone())
            .or_insert_with(HashSet::new)
            .insert(symbol.line_number);
    }
    lines
}

fn build_symbol_reference_from_name(
    file: &IndexedFile,
    line_number: usize,
    symbol_name: &str,
    symbol_lookup: &BTreeMap<String, Vec<&IndexedSymbol>>,
    context: ReferenceContext,
    reason: &str,
    default_confidence: EdgeConfidence,
) -> Option<IndexedSymbolReference> {
    let targets = symbol_lookup.get(symbol_name)?;
    if targets.len() != 1 {
        return None;
    }

    let target = targets[0];
    Some(IndexedSymbolReference {
        from_file: file.path.clone(),
        symbol_name: symbol_name.to_string(),
        target_file: Some(target.file_path.clone()),
        target_line: Some(target.line_number),
        line_number,
        confidence: default_confidence,
        reason: reason.to_string(),
        context,
        additional_count: 0,
    })
}

fn insert_grouped_reference(
    grouped: &mut BTreeMap<
        (
            String,
            String,
            Option<String>,
            ReferenceContext,
            EdgeConfidence,
            String,
        ),
        IndexedSymbolReference,
    >,
    reference: IndexedSymbolReference,
) {
    let key = reference_key(&reference);
    grouped
        .entry(key)
        .and_modify(|existing| {
            existing.additional_count += reference.additional_count + 1;
            if reference.line_number < existing.line_number {
                existing.line_number = reference.line_number;
            }
            if existing.target_line.is_none() {
                existing.target_line = reference.target_line;
            }
        })
        .or_insert(reference);
}

fn reference_key(
    reference: &IndexedSymbolReference,
) -> (
    String,
    String,
    Option<String>,
    ReferenceContext,
    EdgeConfidence,
    String,
) {
    (
        reference.from_file.clone(),
        reference.symbol_name.clone(),
        reference.target_file.clone(),
        reference.context,
        reference.confidence.clone(),
        reference.reason.clone(),
    )
}

fn is_referenceable_symbol_kind(kind: &SymbolKind) -> bool {
    matches!(
        kind,
        SymbolKind::Struct
            | SymbolKind::Enum
            | SymbolKind::Trait
            | SymbolKind::TypeAlias
            | SymbolKind::Const
            | SymbolKind::Static
            | SymbolKind::Module
    )
}

fn contains_identifier_reference(line: &str, name: &str) -> bool {
    let mut remainder = line;
    while let Some(index) = remainder.find(name) {
        let absolute = line.len() - remainder.len() + index;
        let before_ok = line[..absolute]
            .chars()
            .next_back()
            .map(|ch| !is_identifier_char(ch))
            .unwrap_or(true);
        let after_index = absolute + name.len();
        let after_ok = line[after_index..]
            .chars()
            .next()
            .map(|ch| !is_identifier_char(ch))
            .unwrap_or(true);
        if before_ok && after_ok {
            return true;
        }
        remainder = &remainder[index + name.len()..];
    }
    false
}

fn is_identifier_char(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

fn classify_rust_reference_context(
    file: &IndexedFile,
    line_number: usize,
    stripped: &str,
    current_function_context: ReferenceContext,
    in_test_section: bool,
) -> ReferenceContext {
    if matches!(file.role, FileRole::Test) || is_test_path(&file.path) {
        return classify_test_reference_line(stripped, current_function_context);
    }

    if !in_test_section || line_number == 0 {
        return ReferenceContext::Production;
    }

    classify_test_reference_line(stripped, current_function_context)
}

fn classify_test_reference_line(
    stripped: &str,
    current_function_context: ReferenceContext,
) -> ReferenceContext {
    if matches!(
        current_function_context,
        ReferenceContext::Fixture | ReferenceContext::Test
    ) {
        return current_function_context;
    }

    if stripped.starts_with("use ") {
        return ReferenceContext::Fixture;
    }

    if stripped.contains("assert!")
        || stripped.contains("assert_eq!")
        || stripped.contains("assert_ne!")
        || stripped.contains("assert_matches!")
        || stripped.contains("matches!")
    {
        return ReferenceContext::Test;
    }

    if stripped.contains(':') && stripped.contains('=') {
        return ReferenceContext::Fixture;
    }

    if stripped.contains("fixture")
        || stripped.contains("helper")
        || stripped.contains("loaded_index")
        || stripped.contains("source_texts")
        || stripped.contains("source_file")
        || stripped.contains("build_")
        || stripped.contains("make_")
        || stripped.contains("repo(")
        || stripped.contains("symbol(")
    {
        return ReferenceContext::Fixture;
    }

    ReferenceContext::Unknown
}

fn is_strong_test_reference_line(stripped: &str) -> bool {
    stripped.starts_with("use ")
        || stripped.contains("::")
        || stripped.contains("assert!")
        || stripped.contains("assert_eq!")
        || stripped.contains("assert_ne!")
        || stripped.contains("assert_matches!")
        || stripped.contains("matches!")
        || (stripped.contains(':') && stripped.contains('='))
}

fn classify_test_function_name(name: &str) -> ReferenceContext {
    let lower = name.to_lowercase();
    if lower.contains("test") {
        ReferenceContext::Test
    } else if lower.contains("fixture")
        || lower.contains("helper")
        || lower.contains("loaded")
        || lower.contains("source")
        || lower.contains("build")
        || lower.contains("make")
        || lower.contains("repo")
        || lower.contains("symbol")
        || lower.contains("setup")
    {
        ReferenceContext::Fixture
    } else {
        ReferenceContext::Unknown
    }
}

fn rust_function_name_from_line(line: &str) -> Option<String> {
    let (_, remainder) = rust_visibility_prefix(line);
    let remainder = strip_rust_item_prefixes(remainder);
    parse_rust_function_symbol(remainder)
}

fn is_test_section_line(line: &str) -> bool {
    line.starts_with("#[cfg(test)]") || line.starts_with("mod tests ") || line == "mod tests {"
}

fn reference_context_priority(context: ReferenceContext) -> usize {
    match context {
        ReferenceContext::Production => 0,
        ReferenceContext::Fixture => 1,
        ReferenceContext::Test => 2,
        ReferenceContext::Unknown => 3,
    }
}

fn reference_confidence_priority(confidence: &EdgeConfidence) -> usize {
    match confidence {
        EdgeConfidence::Extracted => 0,
        EdgeConfidence::Inferred => 1,
        EdgeConfidence::Ambiguous => 2,
    }
}

fn rust_symbol_from_line(file_path: &str, line_number: usize, line: &str) -> Option<IndexedSymbol> {
    let stripped = strip_line_comment(line).trim();
    if stripped.is_empty() {
        return None;
    }

    let signature = Some(shorten_snippet(stripped, 120));
    let (visibility, remainder) = rust_visibility_prefix(stripped);
    let remainder = strip_rust_item_prefixes(remainder);

    if let Some(name) = parse_rust_module_symbol(remainder) {
        return Some(symbol_record(
            file_path,
            line_number,
            name,
            SymbolKind::Module,
            visibility,
            signature,
        ));
    }
    if let Some(name) = parse_rust_function_symbol(remainder) {
        return Some(symbol_record(
            file_path,
            line_number,
            name,
            SymbolKind::Function,
            visibility,
            signature,
        ));
    }
    if let Some(name) = parse_rust_struct_symbol(remainder) {
        return Some(symbol_record(
            file_path,
            line_number,
            name,
            SymbolKind::Struct,
            visibility,
            signature,
        ));
    }
    if let Some(name) = parse_rust_enum_symbol(remainder) {
        return Some(symbol_record(
            file_path,
            line_number,
            name,
            SymbolKind::Enum,
            visibility,
            signature,
        ));
    }
    if let Some(name) = parse_rust_trait_symbol(remainder) {
        return Some(symbol_record(
            file_path,
            line_number,
            name,
            SymbolKind::Trait,
            visibility,
            signature,
        ));
    }
    if let Some(name) = parse_rust_impl_symbol(remainder) {
        return Some(symbol_record(
            file_path,
            line_number,
            name,
            SymbolKind::Impl,
            Visibility::Private,
            signature,
        ));
    }
    if let Some(name) = parse_rust_type_alias_symbol(remainder) {
        return Some(symbol_record(
            file_path,
            line_number,
            name,
            SymbolKind::TypeAlias,
            visibility,
            signature,
        ));
    }
    if let Some(name) = parse_rust_const_symbol(remainder) {
        return Some(symbol_record(
            file_path,
            line_number,
            name,
            SymbolKind::Const,
            visibility,
            signature,
        ));
    }
    if let Some(name) = parse_rust_static_symbol(remainder) {
        return Some(symbol_record(
            file_path,
            line_number,
            name,
            SymbolKind::Static,
            visibility,
            signature,
        ));
    }

    None
}

fn symbol_record(
    file_path: &str,
    line_number: usize,
    name: String,
    kind: SymbolKind,
    visibility: Visibility,
    signature: Option<String>,
) -> IndexedSymbol {
    IndexedSymbol {
        name,
        kind,
        file_path: file_path.to_string(),
        line_number,
        visibility,
        signature,
    }
}

pub(crate) fn rust_visibility_prefix(line: &str) -> (Visibility, &str) {
    let trimmed = line.trim_start();
    if let Some(rest) = trimmed.strip_prefix("pub(crate)") {
        return (Visibility::Public, rest.trim_start());
    }
    if let Some(rest) = trimmed.strip_prefix("pub(super)") {
        return (Visibility::Public, rest.trim_start());
    }
    if let Some(rest) = trimmed.strip_prefix("pub(in ") {
        if let Some(close) = rest.find(')') {
            return (Visibility::Public, rest[close + 1..].trim_start());
        }
    }
    if let Some(rest) = trimmed.strip_prefix("pub ") {
        return (Visibility::Public, rest.trim_start());
    }
    (Visibility::Private, trimmed)
}

fn strip_rust_item_prefixes(line: &str) -> &str {
    let mut current = line.trim_start();
    loop {
        if let Some(rest) = current.strip_prefix("async ") {
            current = rest.trim_start();
            continue;
        }
        if let Some(rest) = current.strip_prefix("unsafe ") {
            current = rest.trim_start();
            continue;
        }
        if let Some(rest) = current.strip_prefix("default ") {
            current = rest.trim_start();
            continue;
        }
        break;
    }
    current
}

pub(crate) fn parse_rust_module_symbol(line: &str) -> Option<String> {
    let remainder = line.strip_prefix("mod ")?;
    let name = remainder.trim().strip_suffix(';')?.trim();
    if is_rust_identifier(name) {
        Some(name.to_string())
    } else {
        None
    }
}

pub(crate) fn parse_rust_function_symbol(line: &str) -> Option<String> {
    let remainder = line.strip_prefix("fn ")?;
    parse_rust_identifier_name(remainder)
}

pub(crate) fn parse_rust_struct_symbol(line: &str) -> Option<String> {
    let remainder = line.strip_prefix("struct ")?;
    parse_rust_identifier_name(remainder)
}

pub(crate) fn parse_rust_enum_symbol(line: &str) -> Option<String> {
    let remainder = line.strip_prefix("enum ")?;
    parse_rust_identifier_name(remainder)
}

pub(crate) fn parse_rust_trait_symbol(line: &str) -> Option<String> {
    let remainder = line.strip_prefix("trait ")?;
    parse_rust_identifier_name(remainder)
}

pub(crate) fn parse_rust_type_alias_symbol(line: &str) -> Option<String> {
    let remainder = line.strip_prefix("type ")?;
    parse_rust_identifier_name(remainder)
}

pub(crate) fn parse_rust_const_symbol(line: &str) -> Option<String> {
    let remainder = line.strip_prefix("const ")?;
    if remainder.trim_start().starts_with("fn ") {
        return None;
    }
    parse_rust_const_like_name(remainder)
}

pub(crate) fn parse_rust_static_symbol(line: &str) -> Option<String> {
    let remainder = line.strip_prefix("static ")?;
    let remainder = remainder.strip_prefix("mut ").unwrap_or(remainder);
    parse_rust_const_like_name(remainder)
}

pub(crate) fn parse_rust_impl_symbol(line: &str) -> Option<String> {
    let remainder = line.strip_prefix("impl ")?;
    parse_rust_impl_name(remainder)
}

fn parse_rust_identifier_name(text: &str) -> Option<String> {
    let trimmed = text.trim_start();
    let (name, _) = read_rust_identifier(trimmed)?;
    Some(name)
}

fn parse_rust_const_like_name(text: &str) -> Option<String> {
    let trimmed = text.trim_start();
    let (name, _) = read_rust_identifier(trimmed)?;
    Some(name)
}

fn parse_rust_impl_name(text: &str) -> Option<String> {
    let mut body = text.trim_start();
    if body.starts_with('<') {
        body = skip_rust_angle_brackets(body);
    }
    let body = body
        .split_once(" where ")
        .map(|(head, _)| head)
        .unwrap_or(body)
        .split_once('{')
        .map(|(head, _)| head)
        .unwrap_or(body)
        .trim();
    if body.is_empty() {
        return Some("impl".to_string());
    }

    if let Some((trait_part, type_part)) = body.split_once(" for ") {
        let trait_name = simplify_rust_symbol_name(trait_part);
        let type_name = simplify_rust_symbol_name(type_part);
        if !trait_name.is_empty() && !type_name.is_empty() {
            return Some(format!("{trait_name} for {type_name}"));
        }
    }

    let name = simplify_rust_symbol_name(body);
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

fn skip_rust_angle_brackets(text: &str) -> &str {
    let mut depth = 0usize;
    for (idx, ch) in text.char_indices() {
        match ch {
            '<' => depth += 1,
            '>' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return text[idx + ch.len_utf8()..].trim_start();
                }
            }
            _ => {}
        }
    }
    text
}

fn simplify_rust_symbol_name(text: &str) -> String {
    let trimmed = text.trim();
    let trimmed = trimmed
        .split_once('<')
        .map(|(head, _)| head)
        .unwrap_or(trimmed);
    let trimmed = trimmed
        .split_once(" where ")
        .map(|(head, _)| head)
        .unwrap_or(trimmed);
    let trimmed = trimmed.trim_end_matches('{').trim_end_matches(';').trim();
    trimmed
        .split_whitespace()
        .next()
        .unwrap_or(trimmed)
        .trim_end_matches(',')
        .to_string()
}

fn read_rust_identifier(text: &str) -> Option<(String, &str)> {
    let mut chars = text.char_indices();
    let (_, first) = chars.next()?;
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return None;
    }

    let mut end = first.len_utf8();
    for (idx, ch) in chars {
        if ch == '_' || ch.is_ascii_alphanumeric() {
            end = idx + ch.len_utf8();
        } else {
            break;
        }
    }

    Some((text[..end].to_string(), &text[end..]))
}

pub(crate) fn build_rust_edges(
    files: &[IndexedFile],
    source_texts: &BTreeMap<String, String>,
) -> Vec<IndexedEdge> {
    let module_lookup = build_rust_module_lookup(files);
    let mut edges = Vec::new();
    let mut seen = HashSet::new();

    for file in files
        .iter()
        .filter(|file| matches!(file.role, FileRole::Source) && file.path.ends_with(".rs"))
    {
        let Some(text) = source_texts.get(&file.path) else {
            continue;
        };
        let module_path = rust_module_path_components(&file.path);
        let imported_modules = rust_imported_module_names(text, &module_path, &module_lookup);

        for edge in rust_declares_module_edges(file, text, &module_path, &module_lookup) {
            push_unique_edge(&mut edges, &mut seen, edge);
        }
        for edge in rust_import_edges(file, text, &module_path, &module_lookup) {
            push_unique_edge(&mut edges, &mut seen, edge);
        }
        for edge in
            rust_reference_edges(file, text, &module_path, &module_lookup, &imported_modules)
        {
            push_unique_edge(&mut edges, &mut seen, edge);
        }
    }

    edges
}

fn build_rust_module_lookup(files: &[IndexedFile]) -> HashMap<String, String> {
    let mut lookup = HashMap::new();

    for file in files
        .iter()
        .filter(|file| matches!(file.role, FileRole::Source) && file.path.ends_with(".rs"))
    {
        if let Some(key) = rust_module_lookup_key(&file.path) {
            lookup.entry(key).or_insert_with(|| file.path.clone());
        }
    }

    lookup
}

fn push_unique_edge(
    edges: &mut Vec<IndexedEdge>,
    seen: &mut HashSet<(String, String, String)>,
    edge: IndexedEdge,
) {
    let key = (edge.edge_type.clone(), edge.from.clone(), edge.to.clone());
    if seen.insert(key) {
        edges.push(edge);
    }
}

fn rust_declares_module_edges(
    file: &IndexedFile,
    text: &str,
    module_path: &[String],
    module_lookup: &HashMap<String, String>,
) -> Vec<IndexedEdge> {
    let mut edges = Vec::new();
    let mut seen = HashSet::new();

    for line in text.lines() {
        let line = strip_line_comment(line).trim();
        if let Some(module_name) = parse_rust_mod_declaration(line) {
            let mut candidate = module_path.to_vec();
            candidate.push(module_name.clone());
            if let Some(target) = resolve_rust_module_path(&candidate, module_lookup) {
                let key = target.clone();
                if seen.insert(key.clone()) {
                    edges.push(IndexedEdge {
                        edge_type: "declares_module".to_string(),
                        from: file.path.clone(),
                        to: target,
                        confidence: EdgeConfidence::Extracted,
                        reason: format!("declares module {module_name}"),
                    });
                }
            }
        }
    }

    edges
}

fn rust_import_edges(
    file: &IndexedFile,
    text: &str,
    module_path: &[String],
    module_lookup: &HashMap<String, String>,
) -> Vec<IndexedEdge> {
    let mut edges = Vec::new();
    let mut seen = HashSet::new();

    for line in text.lines() {
        let line = strip_line_comment(line).trim();
        if let Some(import_body) = parse_rust_use_statement(line) {
            for path in expand_rust_use_paths(import_body) {
                if let Some((target_path, confidence)) =
                    resolve_rust_use_path(&path, module_path, module_lookup)
                {
                    if seen.insert(target_path.clone()) {
                        let target_display = rust_module_display_path_from_path(&target_path)
                            .unwrap_or_else(|| path.join("::"));
                        edges.push(IndexedEdge {
                            edge_type: "imports".to_string(),
                            from: file.path.clone(),
                            to: target_path,
                            confidence,
                            reason: format!("imports {target_display}"),
                        });
                    }
                }
            }
        }
    }

    edges
}

fn rust_reference_edges(
    file: &IndexedFile,
    text: &str,
    _module_path: &[String],
    module_lookup: &HashMap<String, String>,
    imported_modules: &HashSet<String>,
) -> Vec<IndexedEdge> {
    let mut edges = Vec::new();
    let mut seen = HashSet::new();
    let local_module_names = rust_local_module_names(module_lookup);

    for line in text.lines() {
        let line = strip_line_comment(line).trim();
        if line.starts_with("use ")
            || line.starts_with("pub use ")
            || line.starts_with("mod ")
            || line.starts_with("pub mod ")
        {
            continue;
        }

        for candidate in rust_reference_candidates(line, "crate::") {
            if let Some((target_path, _)) = resolve_rust_use_path(&candidate, &[], module_lookup) {
                if seen.insert(target_path.clone()) {
                    let target_display = rust_module_display_path_from_path(&target_path)
                        .unwrap_or_else(|| candidate.join("::"));
                    edges.push(IndexedEdge {
                        edge_type: "references".to_string(),
                        from: file.path.clone(),
                        to: target_path,
                        confidence: EdgeConfidence::Extracted,
                        reason: format!("references {target_display}"),
                    });
                }
            }
        }

        for module_name in imported_modules.iter().chain(local_module_names.iter()) {
            if !contains_bare_module_reference(line, module_name) {
                continue;
            }
            let candidate = vec![module_name.clone()];
            if let Some(target_path) = resolve_rust_module_path(&candidate, module_lookup) {
                if seen.insert(target_path.clone()) {
                    let target_display = rust_module_display_path_from_path(&target_path)
                        .unwrap_or_else(|| module_name.clone());
                    edges.push(IndexedEdge {
                        edge_type: "references".to_string(),
                        from: file.path.clone(),
                        to: target_path,
                        confidence: if imported_modules.contains(module_name) {
                            EdgeConfidence::Extracted
                        } else {
                            EdgeConfidence::Inferred
                        },
                        reason: if imported_modules.contains(module_name) {
                            format!("references {target_display}")
                        } else {
                            format!("references {target_display}")
                        },
                    });
                }
            }
        }
    }

    edges
}

fn rust_imported_module_names(
    text: &str,
    module_path: &[String],
    module_lookup: &HashMap<String, String>,
) -> HashSet<String> {
    let mut modules = HashSet::new();

    for line in text.lines() {
        let line = strip_line_comment(line).trim();
        if let Some(import_body) = parse_rust_use_statement(line) {
            for path in expand_rust_use_paths(import_body) {
                if let Some((target_path, _)) =
                    resolve_rust_use_path(&path, module_path, module_lookup)
                {
                    if let Some(name) = rust_module_name_from_path(&target_path) {
                        modules.insert(name);
                    }
                }
            }
        }
    }

    modules
}

fn rust_local_module_names(module_lookup: &HashMap<String, String>) -> HashSet<String> {
    module_lookup
        .keys()
        .filter_map(|key| key.split('/').next().map(|part| part.to_string()))
        .collect()
}

fn rust_module_lookup_key(path: &str) -> Option<String> {
    let normalized = path.replace('\\', "/");
    let stripped = if let Some(value) = normalized.strip_prefix("src/") {
        value.to_string()
    } else {
        normalized
    };

    let key = if let Some(value) = stripped.strip_suffix("/mod.rs") {
        value.to_string()
    } else if let Some(value) = stripped.strip_suffix(".rs") {
        value.to_string()
    } else {
        return None;
    };

    if key.is_empty() {
        None
    } else {
        Some(key)
    }
}

pub(crate) fn rust_module_path_components(path: &str) -> Vec<String> {
    let normalized = path.replace('\\', "/");
    let stripped = if let Some(value) = normalized.strip_prefix("src/") {
        value.to_string()
    } else {
        normalized
    };

    let module_path = if stripped == "main.rs" || stripped == "lib.rs" {
        String::new()
    } else if let Some(value) = stripped.strip_suffix("/mod.rs") {
        value.to_string()
    } else if let Some(value) = stripped.strip_suffix(".rs") {
        value.to_string()
    } else {
        stripped
    };

    module_path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(|segment| segment.to_string())
        .collect()
}

pub(crate) fn rust_module_name_from_path(path: &str) -> Option<String> {
    rust_module_lookup_key(path).and_then(|key| key.split('/').next().map(|part| part.to_string()))
}

pub(crate) fn rust_module_display_path_from_path(path: &str) -> Option<String> {
    let components = rust_module_path_components(path);
    if components.is_empty() {
        None
    } else {
        Some(format!("crate::{}", components.join("::")))
    }
}

pub(crate) fn resolve_rust_module_path(
    candidate: &[String],
    module_lookup: &HashMap<String, String>,
) -> Option<String> {
    for end in (1..=candidate.len()).rev() {
        let key = candidate[..end].join("/");
        if let Some(path) = module_lookup.get(&key) {
            return Some(path.clone());
        }
    }

    None
}

pub(crate) fn resolve_rust_use_path(
    candidate: &[String],
    current_module_path: &[String],
    module_lookup: &HashMap<String, String>,
) -> Option<(String, EdgeConfidence)> {
    if candidate.is_empty() {
        return None;
    }

    let (base, relative) = match candidate.first().map(|part| part.as_str()) {
        Some("crate") => (&[][..], &candidate[1..]),
        Some("super") => {
            let parent_len = current_module_path.len().saturating_sub(1);
            (&current_module_path[..parent_len], &candidate[1..])
        }
        Some("self") => (current_module_path, &candidate[1..]),
        _ => (&[][..], candidate),
    };

    let mut combined = base.to_vec();
    combined.extend(relative.iter().cloned());

    resolve_rust_module_path(&combined, module_lookup).map(|target| {
        let confidence = if matches!(candidate.first().map(|part| part.as_str()), Some("crate")) {
            EdgeConfidence::Extracted
        } else {
            EdgeConfidence::Extracted
        };
        (target, confidence)
    })
}

pub(crate) fn parse_rust_mod_declaration(line: &str) -> Option<String> {
    let (_, trimmed) = rust_visibility_prefix(line);
    let remainder = trimmed.strip_prefix("mod ")?;
    let name = remainder.strip_suffix(';')?.trim();
    if is_rust_identifier(name) {
        Some(name.to_string())
    } else {
        None
    }
}

pub(crate) fn parse_rust_use_statement(line: &str) -> Option<&str> {
    let (_, trimmed) = rust_visibility_prefix(line);
    trimmed
        .strip_prefix("use ")?
        .strip_suffix(';')
        .map(str::trim)
}

pub(crate) fn expand_rust_use_paths(body: &str) -> Vec<Vec<String>> {
    let body = body.trim();
    if body.is_empty() {
        return Vec::new();
    }

    if let Some(open_brace) = body.find('{') {
        if let Some(close_brace) = matching_brace(body, open_brace) {
            let mut prefix = body[..open_brace].trim();
            prefix = prefix.strip_suffix("::").unwrap_or(prefix).trim();
            let prefix_segments = parse_rust_path_segments(prefix);
            let inner = &body[open_brace + 1..close_brace];
            let mut paths = Vec::new();
            for part in split_top_level_commas(inner) {
                let item = part.trim();
                if item.is_empty() {
                    continue;
                }
                let mut segments = prefix_segments.clone();
                segments.extend(parse_rust_path_segments(strip_rust_alias(item)));
                if !segments.is_empty() {
                    paths.push(segments);
                }
            }
            return paths;
        }
    }

    let segments = parse_rust_path_segments(strip_rust_alias(body));
    if segments.is_empty() {
        Vec::new()
    } else {
        vec![segments]
    }
}

pub(crate) fn parse_rust_path_segments(text: &str) -> Vec<String> {
    text.split("::")
        .filter_map(|part| {
            let trimmed = part.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
        .collect()
}

pub(crate) fn strip_rust_alias(text: &str) -> &str {
    text.split_once(" as ")
        .map(|(value, _)| value)
        .unwrap_or(text)
}

pub(crate) fn split_top_level_commas(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut depth = 0usize;

    for (idx, ch) in text.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(text[start..idx].trim());
                start = idx + ch.len_utf8();
            }
            _ => {}
        }
    }

    parts.push(text[start..].trim());
    parts
}

pub(crate) fn matching_brace(text: &str, open_idx: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (idx, ch) in text.char_indices().skip_while(|(idx, _)| *idx < open_idx) {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(idx);
                }
            }
            _ => {}
        }
    }
    None
}

fn strip_line_comment(line: &str) -> &str {
    line.split_once("//").map(|(head, _)| head).unwrap_or(line)
}

fn is_rust_identifier(text: &str) -> bool {
    let mut chars = text.chars();
    match chars.next() {
        Some(ch) if ch == '_' || ch.is_ascii_alphabetic() => {}
        _ => return false,
    }

    chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn rust_reference_candidates(line: &str, prefix: &str) -> Vec<Vec<String>> {
    let mut candidates = Vec::new();
    let mut remainder = line;

    while let Some(index) = remainder.find(prefix) {
        let after = &remainder[index + prefix.len()..];
        let chain = take_rust_path_chain(after);
        if !chain.is_empty() {
            let segments = parse_rust_path_segments(chain);
            if !segments.is_empty() {
                candidates.push(segments);
            }
        }
        if chain.len() >= after.len() {
            break;
        }
        remainder = &after[chain.len()..];
    }

    candidates
}

fn take_rust_path_chain(text: &str) -> &str {
    let mut end = 0usize;
    for (idx, ch) in text.char_indices() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == ':' {
            end = idx + ch.len_utf8();
            continue;
        }
        break;
    }
    &text[..end]
}

fn contains_bare_module_reference(line: &str, module_name: &str) -> bool {
    let needle = format!("{module_name}::");
    let mut remainder = line;
    while let Some(index) = remainder.find(&needle) {
        let absolute = line.len() - remainder.len() + index;
        let start_ok = line[..absolute]
            .chars()
            .next_back()
            .map(|ch| !ch.is_ascii_alphanumeric() && ch != '_')
            .unwrap_or(true);
        if start_ok {
            return true;
        }
        remainder = &remainder[index + needle.len()..];
    }
    false
}

fn choose_source_roots(source_paths: &[String]) -> Vec<String> {
    let mut roots = Vec::new();
    for candidate in [
        "src/main.rs",
        "src/lib.rs",
        "app/main.py",
        "src/index.ts",
        "src/index.js",
    ] {
        if source_paths.iter().any(|path| path == candidate) {
            roots.push(candidate.to_string());
        }
    }
    if roots.is_empty() {
        if let Some(first) = source_paths.first() {
            roots.push(first.clone());
        }
    }
    roots
}

fn count_roles(files: &[IndexedFile]) -> BTreeMap<FileRole, usize> {
    let mut counts = BTreeMap::new();
    for file in files {
        *counts.entry(file.role.clone()).or_insert(0) += 1;
    }
    counts
}

fn collect_files(
    root: &Path,
    out: &mut Vec<IndexedFile>,
    source_texts: &mut BTreeMap<String, String>,
    lex_texts: &mut BTreeMap<String, String>,
) -> Result<()> {
    let walker = WalkBuilder::new(root)
        .standard_filters(true)
        .add_custom_ignore_filename(".rgignore")
        .build();

    for entry in walker {
        let entry = entry?;
        let path = entry.path();
        if !entry
            .file_type()
            .map(|value| value.is_file())
            .unwrap_or(false)
        {
            continue;
        }
        if should_skip_file(path) {
            continue;
        }

        let relative = path.strip_prefix(root).unwrap_or(path);
        let relative_path = display_path(relative);
        let role = classify_role(&relative_path);
        let metadata = entry.metadata()?;
        let size_bytes = Some(metadata.len());
        let modified_unix = metadata.modified().ok().and_then(system_time_to_unix);
        let content_hash = maybe_hash_file(path, metadata.len())?;

        out.push(IndexedFile {
            path: relative_path.clone(),
            role,
            size_bytes,
            modified_unix,
            content_hash,
            lex_stats: None,
        });

        if crate::parser::language::detect_language(&relative_path).is_some() {
            let contents = fs::read_to_string(path)
                .with_context(|| format!("failed to read source file {}", display_path(path)))?;
            source_texts.insert(relative_path.clone(), contents.clone());
            lex_texts.insert(relative_path, contents);
        } else if metadata.len() <= LEX_READ_SIZE_LIMIT {
            if let Ok(contents) = fs::read_to_string(path) {
                lex_texts.insert(relative_path, contents);
            }
        }
    }

    Ok(())
}

fn maybe_hash_file(path: &Path, size_bytes: u64) -> Result<Option<String>> {
    if size_bytes > HASH_LIMIT_BYTES {
        return Ok(None);
    }

    let mut file =
        File::open(path).with_context(|| format!("failed to open {}", display_path(path)))?;
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer)?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    buffer.hash(&mut hasher);
    Ok(Some(format!("{:016x}", hasher.finish())))
}

fn system_time_to_unix(time: SystemTime) -> Option<u64> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs())
}

fn determine_state(repo: &RepoInfo, index: Option<&RepoIndex>) -> IndexState {
    let Some(index) = index else {
        return IndexState::Missing;
    };

    match (&repo.rev, &index.repo_rev) {
        (Some(current), Some(indexed)) if current == indexed => IndexState::Fresh,
        (Some(_), Some(_)) => IndexState::Stale,
        (Some(_), None) => IndexState::Stale,
        (None, _) => IndexState::Unverifiable,
    }
}

fn write_index_file(index_path: &Path, index: &RepoIndex) -> Result<()> {
    if let Some(parent) = index_path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!("failed to create index directory {}", display_path(parent))
        })?;
    }

    let data = serde_json::to_string_pretty(index)?;
    let mut file = File::create(index_path)
        .with_context(|| format!("failed to create index file {}", display_path(index_path)))?;
    file.write_all(data.as_bytes())?;
    file.write_all(b"\n")?;
    Ok(())
}

fn read_index_file(index_path: &Path) -> Result<Option<RepoIndex>> {
    if !index_path.exists() {
        return Ok(None);
    }

    let data = fs::read_to_string(index_path)
        .with_context(|| format!("failed to read index file {}", display_path(index_path)))?;
    let index = serde_json::from_str(&data)
        .with_context(|| format!("failed to parse index file {}", display_path(index_path)))?;
    Ok(Some(index))
}

fn remove_empty_parent(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty() || !path.exists() {
        return Ok(());
    }

    if fs::read_dir(path)
        .map(|mut entries| entries.next().is_none())
        .unwrap_or(false)
    {
        fs::remove_dir(path).ok();
    }
    Ok(())
}

fn format_role_counts(counts: &BTreeMap<FileRole, usize>) -> String {
    if counts.is_empty() {
        return "none".to_string();
    }
    counts
        .iter()
        .map(|(role, count)| format!("{role}:{count}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn format_symbol_kind_counts(counts: &BTreeMap<SymbolKind, usize>) -> String {
    if counts.is_empty() {
        return "none".to_string();
    }
    counts
        .iter()
        .map(|(kind, count)| format!("{kind}:{count}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn should_skip_file(path: &Path) -> bool {
    let lower = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_lowercase();
    lower.ends_with(".exe")
        || lower.ends_with(".dll")
        || lower.ends_with(".pdb")
        || path.components().any(|component| {
            component
                .as_os_str()
                .to_string_lossy()
                .eq_ignore_ascii_case(".git")
        })
}

fn split_path(path: &str) -> Vec<&str> {
    path.split('/')
        .filter(|segment| !segment.is_empty())
        .collect()
}

fn file_stem(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(path)
        .to_string()
}

fn test_stem(path: &str) -> String {
    let stem = file_stem(path);
    stem.trim_start_matches("test_")
        .trim_end_matches("_test")
        .trim_end_matches(".test")
        .trim_end_matches(".spec")
        .to_string()
}

fn path_tokens(text: &str) -> Vec<String> {
    text.split(|ch: char| !ch.is_alphanumeric())
        .filter_map(|part| {
            let part = part.to_lowercase();
            if part.is_empty() {
                None
            } else {
                Some(part)
            }
        })
        .collect()
}

fn shared_token_count(left: &[String], right: &[String]) -> usize {
    let right_set: BTreeSet<&String> = right.iter().collect();
    left.iter().filter(|term| right_set.contains(term)).count()
}

fn is_source_path(path: &str) -> bool {
    let lower = path.to_lowercase();
    let source_extensions = [
        ".rs", ".ts", ".tsx", ".js", ".jsx", ".py", ".go", ".java", ".c", ".cc", ".cpp", ".h",
        ".hpp", ".cs", ".rb", ".php", ".swift", ".kt", ".kts", ".scala", ".sh", ".sql", ".html",
        ".css", ".scss",
    ];

    source_extensions.iter().any(|ext| lower.ends_with(ext))
        || lower.contains("/src/")
        || lower.contains("/app/")
        || lower.contains("/lib/")
        || lower.contains("/services/")
        || lower.contains("/routes/")
        || lower.contains("/pages/")
        || lower.contains("/components/")
        || lower.contains("/server/")
}

fn is_test_path(path: &str) -> bool {
    path.contains("/test")
        || path.contains("/tests")
        || path.contains("__tests__")
        || path.contains("test_")
        || path.ends_with("_test.rs")
        || path.ends_with(".spec.")
        || path.ends_with(".test.")
}

fn is_config_path(path: &str) -> bool {
    path.contains("config")
        || path.contains("settings")
        || path.ends_with(".toml")
        || path.ends_with(".yaml")
        || path.ends_with(".yml")
        || path.ends_with(".json")
        || path.ends_with(".jsonc")
}

fn is_doc_path(path: &str) -> bool {
    path.ends_with(".md")
        || path.ends_with(".rst")
        || path.contains("/docs/")
        || path.contains("/doc/")
        || path.ends_with("readme")
        || path.ends_with("readme.md")
}

fn is_lockfile(path: &str) -> bool {
    let lower = path.to_lowercase();
    lower.ends_with("cargo.lock")
        || lower.ends_with("package-lock.json")
        || lower.ends_with("pnpm-lock.yaml")
        || lower.ends_with("yarn.lock")
        || lower.ends_with("poetry.lock")
        || lower.ends_with("composer.lock")
}

fn is_generated_path(path: &str) -> bool {
    path.contains("/target/")
        || path.contains("/dist/")
        || path.contains("/build/")
        || path.contains("/vendor/")
        || path.contains("generated")
        || is_generated_site_path(path)
}

fn is_generated_site_path(path: &str) -> bool {
    let lower = path.to_lowercase();
    (lower.starts_with("site/") || lower.contains("/site/")) && lower.ends_with("index.html")
}

fn is_browser_extension_manifest(path: &str) -> bool {
    path.to_lowercase().ends_with("manifest.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn source_file(path: &str) -> IndexedFile {
        IndexedFile {
            path: path.to_string(),
            role: FileRole::Source,
            size_bytes: None,
            modified_unix: None,
            content_hash: None,
            lex_stats: None,
        }
    }

    fn source_texts(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
        entries
            .iter()
            .map(|(path, text)| (path.to_string(), text.to_string()))
            .collect()
    }

    fn unique_temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "agentgrep-{}-{}-{}",
            name,
            std::process::id(),
            nanos
        ))
    }

    fn write_file(root: &Path, relative: &str, contents: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    fn indexed_paths(index: &RepoIndex) -> HashSet<String> {
        index.files.iter().map(|file| file.path.clone()).collect()
    }

    #[test]
    fn index_path_prefers_git_dir_when_present() {
        let repo = RepoInfo {
            root: PathBuf::from("C:/repo"),
            rev: None,
            git_dir: Some(PathBuf::from("C:/repo/.git")),
        };
        assert_eq!(
            index_path(&repo),
            PathBuf::from("C:/repo/.git")
                .join("agentgrep")
                .join("index.json")
        );
    }

    #[test]
    fn index_path_falls_back_without_git() {
        let repo = RepoInfo {
            root: PathBuf::from("C:/repo"),
            rev: None,
            git_dir: None,
        };
        assert_eq!(
            index_path(&repo),
            PathBuf::from("C:/repo/.agentgrep/index.json")
        );
    }

    #[test]
    fn classifies_roles() {
        assert_eq!(classify_role("src/main.rs"), FileRole::Source);
        assert_eq!(classify_role("tests/main_test.rs"), FileRole::Test);
        assert_eq!(classify_role("docs/README.md"), FileRole::Doc);
        assert_eq!(classify_role("Cargo.toml"), FileRole::Config);
        assert_eq!(classify_role("Cargo.lock"), FileRole::Lockfile);
        assert_eq!(classify_role("site/docs/index.html"), FileRole::Generated);
    }

    #[test]
    fn skips_gitignored_directories_during_indexing() {
        let base = unique_temp_dir("gitignore-index");
        fs::create_dir_all(base.join(".git")).unwrap();
        write_file(&base, ".gitignore", "target/\n");
        write_file(&base, "src/visible.rs", "pub fn visible() {}\n");
        write_file(&base, "target/ignored.rs", "pub fn ignored() {}\n");

        let repo = RepoInfo {
            root: base.clone(),
            rev: Some("abc".to_string()),
            git_dir: Some(base.join(".git")),
        };

        let index = build_index(&repo).unwrap();
        let paths = indexed_paths(&index);
        assert!(paths.contains("src/visible.rs"));
        assert!(!paths.contains("target/ignored.rs"));
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn builds_symbol_references_for_direct_import_bindings() {
        let base = unique_temp_dir("import-bindings");
        fs::create_dir_all(base.join(".git")).unwrap();
        write_file(&base, "app/llm_client.py", "class LLMClient:\n    pass\n");
        write_file(
            &base,
            "app/meeting_session.py",
            "from app.llm_client import LLMClient\n\nasync def start_session():\n    return LLMClient()\n",
        );

        let repo = RepoInfo {
            root: base.clone(),
            rev: Some("abc".to_string()),
            git_dir: Some(base.join(".git")),
        };

        let index = build_index(&repo).unwrap();
        assert!(index
            .symbols
            .iter()
            .any(|symbol| symbol.name == "LLMClient" && symbol.file_path == "app/llm_client.py"));
        assert!(index.symbol_references.iter().any(|reference| {
            reference.from_file == "app/meeting_session.py"
                && reference.symbol_name == "LLMClient"
                && reference.target_file.as_deref() == Some("app/llm_client.py")
        }));
        assert!(index.stats.symbol_reference_count > 0);
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn builds_manifest_edges_for_background_and_content_scripts() {
        let base = unique_temp_dir("manifest-edges");
        fs::create_dir_all(base.join(".git")).unwrap();
        write_file(
            &base,
            "manifest.json",
            r#"{
  "background": { "service_worker": "src/background/serviceWorker.ts" },
  "content_scripts": [
    {
      "js": ["src/content/contentScript.ts"],
      "css": ["src/content/contentScript.css"]
    }
  ]
}"#,
        );
        write_file(&base, "src/background/serviceWorker.ts", "export {};\n");
        write_file(&base, "src/content/contentScript.ts", "export {};\n");
        write_file(&base, "src/content/contentScript.css", "body {}\n");

        let repo = RepoInfo {
            root: base.clone(),
            rev: Some("abc".to_string()),
            git_dir: Some(base.join(".git")),
        };

        let index = build_index(&repo).unwrap();
        assert!(index.edges.iter().any(|edge| {
            edge.from == "manifest.json"
                && edge.to == "src/background/serviceWorker.ts"
                && edge.edge_type == "references"
        }));
        assert!(index.edges.iter().any(|edge| {
            edge.from == "manifest.json"
                && edge.to == "src/content/contentScript.ts"
                && edge.edge_type == "references"
        }));
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn respects_rgignore_for_nested_ignored_dirs() {
        let base = unique_temp_dir("rgignore-index");
        write_file(&base, "nested/.rgignore", "ignored/\n");
        write_file(&base, "nested/src/keep.rs", "pub fn keep() {}\n");
        write_file(&base, "nested/ignored/skip.rs", "pub fn skip() {}\n");

        let repo = RepoInfo {
            root: base.clone(),
            rev: Some("abc".to_string()),
            git_dir: None,
        };

        let index = build_index(&repo).unwrap();
        let paths = indexed_paths(&index);
        assert!(paths.contains("nested/src/keep.rs"));
        assert!(!paths.contains("nested/ignored/skip.rs"));
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn detects_likely_test_connections() {
        let source = vec!["src/session.rs".to_string(), "src/router.rs".to_string()];
        let targets = likely_test_targets("tests/session_test.rs", &source);
        assert!(targets
            .iter()
            .any(|(target, _, _)| target == "src/session.rs"));
    }

    #[test]
    fn extracts_rust_functions() {
        let files = vec![source_file("src/search.rs")];
        let texts = source_texts(&[("src/search.rs", "pub async fn run() {}\nfn helper() {}\n")]);

        let symbols = build_rust_symbols(&files, &texts);
        assert!(symbols.iter().any(|symbol| {
            symbol.kind == SymbolKind::Function
                && symbol.name == "run"
                && symbol.visibility == Visibility::Public
        }));
        assert!(symbols.iter().any(|symbol| {
            symbol.kind == SymbolKind::Function
                && symbol.name == "helper"
                && symbol.visibility == Visibility::Private
        }));
    }

    #[test]
    fn extracts_structs_and_enums() {
        let files = vec![source_file("src/types.rs")];
        let texts = source_texts(&[("src/types.rs", "pub struct SearchMatch {}\nenum Mode {}\n")]);

        let symbols = build_rust_symbols(&files, &texts);
        assert!(symbols.iter().any(|symbol| {
            symbol.kind == SymbolKind::Struct
                && symbol.name == "SearchMatch"
                && symbol.visibility == Visibility::Public
        }));
        assert!(symbols.iter().any(|symbol| {
            symbol.kind == SymbolKind::Enum
                && symbol.name == "Mode"
                && symbol.visibility == Visibility::Private
        }));
    }

    #[test]
    fn extracts_impl_blocks() {
        let files = vec![source_file("src/search.rs")];
        let texts = source_texts(&[(
            "src/search.rs",
            "impl SearchReport {}\nimpl Display for SearchReport {}\n",
        )]);

        let symbols = build_rust_symbols(&files, &texts);
        assert!(symbols.iter().any(|symbol| {
            symbol.kind == SymbolKind::Impl
                && symbol.name.contains("SearchReport")
                && symbol
                    .signature
                    .as_deref()
                    .unwrap_or_default()
                    .starts_with("impl")
        }));
    }

    #[test]
    fn extracts_modules() {
        let files = vec![source_file("src/main.rs"), source_file("src/search.rs")];
        let texts = source_texts(&[("src/main.rs", "mod search;\npub mod types;\n")]);

        let symbols = build_rust_symbols(&files, &texts);
        assert!(symbols.iter().any(|symbol| {
            symbol.kind == SymbolKind::Module
                && symbol.name == "search"
                && symbol.visibility == Visibility::Private
        }));
        assert!(symbols.iter().any(|symbol| {
            symbol.kind == SymbolKind::Module
                && symbol.name == "types"
                && symbol.visibility == Visibility::Public
        }));
    }

    #[test]
    fn detects_rust_module_declarations() {
        let files = vec![source_file("src/main.rs"), source_file("src/search.rs")];
        let texts = source_texts(&[("src/main.rs", "mod search;")]);

        let edges = build_rust_edges(&files, &texts);
        assert!(edges.iter().any(|edge| {
            edge.edge_type == "declares_module"
                && edge.from == "src/main.rs"
                && edge.to == "src/search.rs"
        }));
    }

    #[test]
    fn detects_crate_type_imports() {
        let files = vec![source_file("src/search.rs"), source_file("src/types.rs")];
        let texts = source_texts(&[("src/search.rs", "use crate::types::SearchMatch;")]);

        let edges = build_rust_edges(&files, &texts);
        assert!(edges.iter().any(|edge| {
            edge.edge_type == "imports" && edge.from == "src/search.rs" && edge.to == "src/types.rs"
        }));
    }

    #[test]
    fn detects_grouped_crate_imports() {
        let files = vec![
            source_file("src/main.rs"),
            source_file("src/search.rs"),
            source_file("src/rank.rs"),
        ];
        let texts = source_texts(&[("src/main.rs", "use crate::{search, rank};")]);

        let edges = build_rust_edges(&files, &texts);
        assert!(edges.iter().any(|edge| {
            edge.edge_type == "imports" && edge.from == "src/main.rs" && edge.to == "src/search.rs"
        }));
        assert!(edges.iter().any(|edge| {
            edge.edge_type == "imports" && edge.from == "src/main.rs" && edge.to == "src/rank.rs"
        }));
    }

    #[test]
    fn extracts_symbol_references_from_rust_use_and_token_lines() {
        let files = vec![source_file("src/search.rs"), source_file("src/types.rs")];
        let texts = source_texts(&[(
            "src/search.rs",
            "pub(crate) use crate::types::SearchMatch;\nfn run() {\n    let _ = SearchMatch;\n}\n",
        ), (
            "src/types.rs",
            "pub struct SearchMatch {}\n",
        )]);

        let symbols = vec![
            IndexedSymbol {
                name: "SearchMatch".to_string(),
                kind: SymbolKind::Struct,
                file_path: "src/types.rs".to_string(),
                line_number: 1,
                visibility: Visibility::Public,
                signature: Some("pub struct SearchMatch {}".to_string()),
            },
            IndexedSymbol {
                name: "run".to_string(),
                kind: SymbolKind::Function,
                file_path: "src/search.rs".to_string(),
                line_number: 2,
                visibility: Visibility::Private,
                signature: Some("fn run() {".to_string()),
            },
        ];

        let references = build_rust_symbol_references(&files, &texts, &symbols);
        assert!(references.iter().any(|reference| {
            reference.from_file == "src/search.rs"
                && reference.symbol_name == "SearchMatch"
                && reference.target_file.as_deref() == Some("src/types.rs")
                && reference.confidence == EdgeConfidence::Extracted
        }));
        assert!(references.iter().any(|reference| {
            reference.from_file == "src/search.rs"
                && reference.symbol_name == "SearchMatch"
                && reference.target_file.as_deref() == Some("src/types.rs")
                && reference.confidence == EdgeConfidence::Inferred
        }));
        assert!(references
            .iter()
            .all(|reference| reference.from_file != "src/types.rs"));
    }

    #[test]
    fn skips_ambiguous_symbol_names() {
        let files = vec![
            source_file("src/search.rs"),
            source_file("src/a.rs"),
            source_file("src/b.rs"),
        ];
        let texts = source_texts(&[
            ("src/search.rs", "use crate::alpha::Helper;\n"),
            ("src/a.rs", "pub struct Helper {}\n"),
            ("src/b.rs", "pub struct Helper {}\n"),
        ]);

        let symbols = vec![
            IndexedSymbol {
                name: "Helper".to_string(),
                kind: SymbolKind::Struct,
                file_path: "src/a.rs".to_string(),
                line_number: 1,
                visibility: Visibility::Public,
                signature: Some("pub struct Helper {}".to_string()),
            },
            IndexedSymbol {
                name: "Helper".to_string(),
                kind: SymbolKind::Struct,
                file_path: "src/b.rs".to_string(),
                line_number: 1,
                visibility: Visibility::Public,
                signature: Some("pub struct Helper {}".to_string()),
            },
        ];

        let references = build_rust_symbol_references(&files, &texts, &symbols);
        assert!(references.is_empty());
    }

    #[test]
    fn dedupes_fixture_references_and_marks_context() {
        let files = vec![source_file("src/search.rs"), source_file("src/types.rs")];
        let texts = source_texts(&[(
            "src/search.rs",
            "#[cfg(test)]\nmod tests {\n    fn loaded_index() {\n        let a: SearchResult = SearchResult {\n        let b: SearchResult = SearchResult {\n    }\n}\n",
        ), (
            "src/types.rs",
            "pub struct SearchResult {}\n",
        )]);

        let symbols = vec![IndexedSymbol {
            name: "SearchResult".to_string(),
            kind: SymbolKind::Struct,
            file_path: "src/types.rs".to_string(),
            line_number: 1,
            visibility: Visibility::Public,
            signature: Some("pub struct SearchResult {}".to_string()),
        }];

        let references = build_rust_symbol_references(&files, &texts, &symbols);
        assert_eq!(references.len(), 1);
        assert_eq!(references[0].context, ReferenceContext::Fixture);
        assert_eq!(references[0].additional_count, 1);
    }

    #[test]
    fn skips_definition_lines_from_reference_extraction() {
        let files = vec![source_file("src/search.rs"), source_file("src/types.rs")];
        let texts = source_texts(&[
            (
                "src/search.rs",
                "fn run() {\n    let _ = SearchResult;\n}\n",
            ),
            ("src/types.rs", "pub struct SearchResult {}\n"),
        ]);

        let symbols = vec![IndexedSymbol {
            name: "SearchResult".to_string(),
            kind: SymbolKind::Struct,
            file_path: "src/types.rs".to_string(),
            line_number: 1,
            visibility: Visibility::Public,
            signature: Some("pub struct SearchResult {}".to_string()),
        }];

        let references = build_rust_symbol_references(&files, &texts, &symbols);
        assert_eq!(references.len(), 1);
        assert_eq!(references[0].line_number, 2);
    }

    #[test]
    fn detects_search_coverage_references_even_with_impl_block() {
        let files = vec![source_file("src/search.rs"), source_file("src/types.rs")];
        let texts = source_texts(&[(
            "src/search.rs",
            "use crate::types::SearchCoverage;\nfn run() {\n    let coverage = SearchCoverage::new();\n    let _: SearchCoverage = coverage;\n}\n",
        ), (
            "src/types.rs",
            "pub struct SearchCoverage {}\nimpl SearchCoverage {\n    pub fn new() -> Self { Self {} }\n}\n",
        )]);

        let symbols = vec![
            IndexedSymbol {
                name: "SearchCoverage".to_string(),
                kind: SymbolKind::Struct,
                file_path: "src/types.rs".to_string(),
                line_number: 1,
                visibility: Visibility::Public,
                signature: Some("pub struct SearchCoverage {}".to_string()),
            },
            IndexedSymbol {
                name: "SearchCoverage".to_string(),
                kind: SymbolKind::Impl,
                file_path: "src/types.rs".to_string(),
                line_number: 2,
                visibility: Visibility::Private,
                signature: Some("impl SearchCoverage {".to_string()),
            },
        ];

        let references = build_rust_symbol_references(&files, &texts, &symbols);
        assert!(references.iter().any(|reference| {
            reference.symbol_name == "SearchCoverage"
                && reference.target_file.as_deref() == Some("src/types.rs")
                && reference.line_number == 1
                && reference.confidence == EdgeConfidence::Extracted
                && matches!(reference.context, ReferenceContext::Production)
        }));
        assert!(references.iter().any(|reference| {
            reference.symbol_name == "SearchCoverage"
                && reference.target_file.as_deref() == Some("src/types.rs")
                && reference.line_number == 3
                && reference.confidence == EdgeConfidence::Inferred
                && reference.additional_count == 1
        }));
    }

    #[test]
    fn keeps_same_area_edges() {
        let files = vec![source_file("src/search.rs"), source_file("src/types.rs")];
        let edges = build_same_area_edges(&files);
        assert!(edges.iter().any(|edge| {
            edge.edge_type == "same_area"
                && edge.from == "src/search.rs"
                && edge.to == "src/types.rs"
        }));
    }

    #[test]
    fn status_logic_handles_missing_fresh_and_stale() {
        let repo = RepoInfo {
            root: PathBuf::from("C:/repo"),
            rev: Some("abc".to_string()),
            git_dir: Some(PathBuf::from("C:/repo/.git")),
        };

        let index = RepoIndex {
            schema_version: INDEX_SCHEMA_VERSION,
            repo_root: "C:/repo".to_string(),
            repo_rev: Some("abc".to_string()),
            indexed_at_unix: 1,
            files: vec![],
            symbols: vec![],
            symbol_references: vec![],
            edges: vec![],
            stats: IndexStats {
                file_count: 0,
                role_counts: BTreeMap::new(),
                symbol_count: 0,
                symbol_kind_counts: BTreeMap::new(),
                symbol_reference_count: 0,
                connection_count: 0,
                ..Default::default()
            },
        };
        assert_eq!(determine_state(&repo, Some(&index)), IndexState::Fresh);

        let stale = RepoIndex {
            repo_rev: Some("def".to_string()),
            ..index.clone()
        };
        assert_eq!(determine_state(&repo, Some(&stale)), IndexState::Stale);
        assert_eq!(determine_state(&repo, None), IndexState::Missing);

        let no_git = RepoInfo {
            root: PathBuf::from("C:/repo"),
            rev: None,
            git_dir: None,
        };
        assert_eq!(
            determine_state(&no_git, Some(&index)),
            IndexState::Unverifiable
        );
    }

    #[test]
    fn serialization_shape_includes_edges_and_stats() {
        let index = RepoIndex {
            schema_version: INDEX_SCHEMA_VERSION,
            repo_root: "C:/repo".to_string(),
            repo_rev: Some("abc".to_string()),
            indexed_at_unix: 1,
            files: vec![IndexedFile {
                path: "src/main.rs".to_string(),
                role: FileRole::Source,
                size_bytes: Some(123),
                modified_unix: Some(456),
                content_hash: Some("deadbeef".to_string()),
                lex_stats: None,
            }],
            symbols: vec![IndexedSymbol {
                name: "main".to_string(),
                kind: SymbolKind::Function,
                file_path: "src/main.rs".to_string(),
                line_number: 1,
                visibility: Visibility::Public,
                signature: Some("pub fn main()".to_string()),
            }],
            symbol_references: vec![],
            edges: vec![IndexedEdge {
                edge_type: "same_area".to_string(),
                from: "src/main.rs".to_string(),
                to: "src/lib.rs".to_string(),
                confidence: EdgeConfidence::Extracted,
                reason: "shared source area src".to_string(),
            }],
            stats: IndexStats {
                file_count: 1,
                role_counts: BTreeMap::from([(FileRole::Source, 1)]),
                symbol_count: 1,
                symbol_kind_counts: BTreeMap::from([(SymbolKind::Function, 1)]),
                symbol_reference_count: 0,
                connection_count: 1,
                ..Default::default()
            },
        };
        let json = serde_json::to_value(&index).unwrap();
        assert_eq!(json["schema_version"], INDEX_SCHEMA_VERSION);
        assert_eq!(json["files"].as_array().unwrap().len(), 1);
        assert_eq!(json["symbols"].as_array().unwrap().len(), 1);
        assert_eq!(json["edges"].as_array().unwrap().len(), 1);
        assert_eq!(json["stats"]["connection_count"], 1);
        assert_eq!(json["stats"]["symbol_count"], 1);
    }

    #[test]
    fn write_read_and_clear_round_trip() {
        let base = unique_temp_dir("index-round-trip");
        let git_dir = base.join(".git");
        fs::create_dir_all(&git_dir).unwrap();
        let repo = RepoInfo {
            root: base.clone(),
            rev: Some("abc".to_string()),
            git_dir: Some(git_dir.clone()),
        };

        let index = RepoIndex {
            schema_version: INDEX_SCHEMA_VERSION,
            repo_root: display_path(&base),
            repo_rev: Some("abc".to_string()),
            indexed_at_unix: 1,
            files: vec![],
            symbols: vec![],
            symbol_references: vec![],
            edges: vec![],
            stats: IndexStats {
                file_count: 0,
                role_counts: BTreeMap::new(),
                symbol_count: 0,
                symbol_kind_counts: BTreeMap::new(),
                symbol_reference_count: 0,
                connection_count: 0,
                ..Default::default()
            },
        };
        let index_file = index_path(&repo);
        write_index_file(&index_file, &index).unwrap();
        assert!(index_file.exists());
        let loaded = read_index_file(&index_file).unwrap().unwrap();
        assert_eq!(loaded.repo_rev.as_deref(), Some("abc"));

        let clear = clear(&repo).unwrap();
        assert!(clear.cleared);
        assert!(!index_file.exists());
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn compute_file_lex_stats_returns_expected_tokens() {
        let stats = compute_file_lex_stats("pub struct SearchResult { name: String }");
        assert!(stats.doc_length > 0, "doc_length should be non-zero");
        assert!(
            stats.term_frequencies.contains_key("search"),
            "should contain 'search' from SearchResult"
        );
        assert!(
            stats.term_frequencies.contains_key("result"),
            "should contain 'result' from SearchResult"
        );
        assert!(
            stats.term_frequencies.contains_key("name"),
            "should contain 'name'"
        );
    }

    #[test]
    fn compute_file_lex_stats_skips_tiny_tokens() {
        let stats = compute_file_lex_stats("fn is it a test");
        assert!(!stats.term_frequencies.contains_key("fn"));
        assert!(!stats.term_frequencies.contains_key("is"));
        assert!(!stats.term_frequencies.contains_key("it"));
        assert!(!stats.term_frequencies.contains_key("a"));
        assert!(stats.term_frequencies.contains_key("test"));
    }

    #[test]
    fn compute_file_lex_stats_skips_pure_numbers() {
        let stats = compute_file_lex_stats("version 123 stable 456");
        assert!(!stats.term_frequencies.contains_key("123"));
        assert!(!stats.term_frequencies.contains_key("456"));
        assert!(stats.term_frequencies.contains_key("version"));
        assert!(stats.term_frequencies.contains_key("stable"));
    }

    #[test]
    fn compute_file_lex_stats_caps_at_max_lex_terms() {
        let text: String = (0..400).map(|i| format!("uniqueterm{i:04} ")).collect();
        let stats = compute_file_lex_stats(&text);
        assert!(
            stats.term_frequencies.len() <= MAX_LEX_TERMS,
            "term_frequencies should be capped at MAX_LEX_TERMS"
        );
    }

    #[test]
    fn compute_file_lex_stats_is_deterministic() {
        let text = "SearchResult IndexedFile FileRole source config test ranking";
        let stats1 = compute_file_lex_stats(text);
        let stats2 = compute_file_lex_stats(text);
        assert_eq!(
            stats1.doc_length, stats2.doc_length,
            "doc_length should be stable"
        );
        let keys1: Vec<_> = stats1.term_frequencies.keys().collect();
        let keys2: Vec<_> = stats2.term_frequencies.keys().collect();
        assert_eq!(keys1, keys2, "term_frequencies keys should be stable");
    }

    #[test]
    fn build_index_populates_lex_stats_for_source_and_doc_files() {
        let base = unique_temp_dir("lex-stats-build");
        fs::create_dir_all(base.join(".git")).unwrap();
        write_file(
            &base,
            "src/main.rs",
            "pub fn main() { let result = SearchResult::new(); }\n",
        );
        write_file(&base, "README.md", "# Agentgrep\nFast local code radar.\n");

        let repo = RepoInfo {
            root: base.clone(),
            rev: Some("abc".to_string()),
            git_dir: Some(base.join(".git")),
        };

        let index = build_index(&repo).unwrap();

        let src = index
            .files
            .iter()
            .find(|f| f.path == "src/main.rs")
            .expect("src/main.rs should be indexed");
        assert!(src.lex_stats.is_some(), "source file should have lex_stats");
        assert!(
            src.lex_stats.as_ref().unwrap().doc_length > 0,
            "source lex_stats doc_length should be non-zero"
        );

        let doc = index
            .files
            .iter()
            .find(|f| f.path == "README.md")
            .expect("README.md should be indexed");
        assert!(doc.lex_stats.is_some(), "doc file should have lex_stats");
        assert!(
            doc.lex_stats.as_ref().unwrap().doc_length > 0,
            "doc lex_stats doc_length should be non-zero"
        );

        assert!(
            index.stats.lex_file_count >= 2,
            "lex_file_count should cover at least both files"
        );
        assert!(
            index.stats.avg_doc_length > 0.0,
            "avg_doc_length should be positive"
        );

        let _ = fs::remove_dir_all(base);
    }
}
