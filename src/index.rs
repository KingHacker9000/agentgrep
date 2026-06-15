use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::repo::{display_path, RepoInfo};

pub const INDEX_SCHEMA_VERSION: u32 = 1;
pub const HASH_LIMIT_BYTES: u64 = 1024 * 256;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoIndex {
    pub schema_version: u32,
    pub repo_root: String,
    pub repo_rev: Option<String>,
    pub indexed_at_unix: u64,
    pub files: Vec<IndexedFile>,
    pub edges: Vec<IndexedEdge>,
    pub stats: IndexStats,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexedFile {
    pub path: String,
    pub role: FileRole,
    pub size_bytes: Option<u64>,
    pub modified_unix: Option<u64>,
    pub content_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexedEdge {
    pub edge_type: String,
    pub from: String,
    pub to: String,
    pub confidence: EdgeConfidence,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexStats {
    pub file_count: usize,
    pub role_counts: BTreeMap<FileRole, usize>,
    pub connection_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum FileRole {
    Source,
    Test,
    Doc,
    Config,
    Lockfile,
    Generated,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    pub connection_count: usize,
}

#[derive(Debug)]
pub struct IndexStatusReport {
    pub index_path: PathBuf,
    pub state: IndexState,
    pub file_count: usize,
    pub role_counts: BTreeMap<FileRole, usize>,
    pub connection_count: usize,
    pub repo_rev: Option<String>,
    pub indexed_rev: Option<String>,
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
        connection_count: index.stats.connection_count,
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
            connection_count: index.stats.connection_count,
            repo_rev: repo.rev.clone(),
            indexed_rev: index.repo_rev,
        })
    } else {
        Ok(IndexStatusReport {
            index_path: loaded.index_path,
            state: loaded.state,
            file_count: 0,
            role_counts: BTreeMap::new(),
            connection_count: 0,
            repo_rev: repo.rev.clone(),
            indexed_rev: None,
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
    println!("- connections counted: {}", report.connection_count);
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
    println!("- connections counted: {}", report.connection_count);
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

fn build_index(repo: &RepoInfo) -> Result<RepoIndex> {
    let mut files = Vec::new();
    collect_files(&repo.root, &repo.root, &mut files)?;
    files.sort_by(|left, right| left.path.cmp(&right.path));

    let source_paths: Vec<String> = files
        .iter()
        .filter(|file| matches!(file.role, FileRole::Source))
        .map(|file| file.path.clone())
        .collect();

    let mut edges = Vec::new();
    edges.extend(build_same_area_edges(&files));
    edges.extend(build_test_edges(&files, &source_paths));
    edges.extend(build_config_edges(&files, &source_paths));

    let file_count = files.len();
    let role_counts = count_roles(&files);
    let indexed_at_unix = unix_now();

    Ok(RepoIndex {
        schema_version: INDEX_SCHEMA_VERSION,
        repo_root: display_path(&repo.root),
        repo_rev: repo.rev.clone(),
        indexed_at_unix,
        files,
        edges: edges.clone(),
        stats: IndexStats {
            file_count,
            role_counts,
            connection_count: edges.len(),
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

fn build_config_edges(files: &[IndexedFile], source_paths: &[String]) -> Vec<IndexedEdge> {
    let mut edges = Vec::new();
    let source_roots = choose_source_roots(source_paths);

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
    }

    edges
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

fn collect_files(root: &Path, current: &Path, out: &mut Vec<IndexedFile>) -> Result<()> {
    for entry in fs::read_dir(current)
        .with_context(|| format!("failed to read directory {}", display_path(current)))?
    {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if should_skip_dir(&name) {
                continue;
            }
            collect_files(root, &path, out)?;
            continue;
        }

        if !path.is_file() || should_skip_file(&path, &name) {
            continue;
        }

        let relative = path.strip_prefix(root).unwrap_or(&path);
        let relative_path = display_path(relative);
        let role = classify_role(&relative_path);
        let metadata = entry.metadata()?;
        let size_bytes = Some(metadata.len());
        let modified_unix = metadata.modified().ok().and_then(system_time_to_unix);
        let content_hash = maybe_hash_file(&path, metadata.len())?;

        out.push(IndexedFile {
            path: relative_path,
            role,
            size_bytes,
            modified_unix,
            content_hash,
        });
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
    counts
        .iter()
        .map(|(role, count)| format!("{role}:{count}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn should_skip_dir(name: &str) -> bool {
    matches!(
        name,
        ".git" | "target" | "node_modules" | "dist" | "build" | "manual-test" | ".agentgrep"
    )
}

fn should_skip_file(path: &Path, name: &str) -> bool {
    let lower = name.to_lowercase();
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

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
            edges: vec![],
            stats: IndexStats {
                file_count: 0,
                role_counts: BTreeMap::new(),
                connection_count: 0,
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
            }],
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
                connection_count: 1,
            },
        };
        let json = serde_json::to_value(&index).unwrap();
        assert_eq!(json["schema_version"], 1);
        assert_eq!(json["files"].as_array().unwrap().len(), 1);
        assert_eq!(json["edges"].as_array().unwrap().len(), 1);
        assert_eq!(json["stats"]["connection_count"], 1);
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
            edges: vec![],
            stats: IndexStats {
                file_count: 0,
                role_counts: BTreeMap::new(),
                connection_count: 0,
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
}
