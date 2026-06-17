use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use tree_sitter::{Language, Parser, Tree};

use crate::index::IndexedFile;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LanguageKind {
    Rust,
    Go,
    Python,
    JavaScript,
    TypeScript,
    Tsx,
}

pub fn detect_language(path: &str) -> Option<LanguageKind> {
    let lower = path.to_lowercase();
    if lower.ends_with(".rs") {
        return Some(LanguageKind::Rust);
    }
    if lower.ends_with(".go") {
        return Some(LanguageKind::Go);
    }
    if lower.ends_with(".py") {
        return Some(LanguageKind::Python);
    }
    if lower.ends_with(".tsx") {
        return Some(LanguageKind::Tsx);
    }
    if lower.ends_with(".ts") || lower.ends_with(".mts") || lower.ends_with(".cts") {
        return Some(LanguageKind::TypeScript);
    }
    if lower.ends_with(".js")
        || lower.ends_with(".jsx")
        || lower.ends_with(".mjs")
        || lower.ends_with(".cjs")
    {
        return Some(LanguageKind::JavaScript);
    }
    None
}

pub fn parse_source(language: Language, source: &str) -> Option<Tree> {
    let mut parser = Parser::new();
    parser.set_language(&language).ok()?;
    parser.parse(source, None)
}

#[derive(Debug, Clone)]
pub struct RepoLookup {
    paths: BTreeSet<String>,
}

impl RepoLookup {
    pub fn new(files: &[IndexedFile]) -> Self {
        let mut paths = BTreeSet::new();
        for file in files {
            paths.insert(normalize_path(&file.path));
        }
        Self { paths }
    }

    pub fn contains_path(&self, path: &str) -> bool {
        self.paths.contains(&normalize_path(path))
    }

    pub fn resolve_candidates<I, S>(&self, candidates: I) -> Option<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        for candidate in candidates {
            let normalized = normalize_path(candidate.as_ref());
            if self.contains_path(&normalized) {
                return Some(normalized);
            }
        }
        None
    }
}

pub fn normalize_path(path: &str) -> String {
    let mut parts = Vec::new();
    let mut prefix = None::<String>;

    for component in Path::new(path).components() {
        match component {
            Component::Prefix(value) => {
                prefix = Some(value.as_os_str().to_string_lossy().to_string())
            }
            Component::RootDir => {}
            Component::CurDir => {}
            Component::ParentDir => {
                parts.pop();
            }
            Component::Normal(part) => parts.push(part.to_string_lossy().to_string()),
        }
    }

    let mut normalized = String::new();
    if let Some(prefix) = prefix {
        normalized.push_str(&prefix);
    }
    if !parts.is_empty() {
        if !normalized.is_empty() {
            normalized.push('/');
        }
        normalized.push_str(&parts.join("/"));
    }
    normalized
}

pub fn parent_dir(path: &str) -> String {
    let normalized = normalize_path(path);
    let parent = Path::new(&normalized).parent().map(PathBuf::from);
    parent
        .map(|value| normalize_path(&value.to_string_lossy()))
        .unwrap_or_default()
}

pub fn join_relative(base_dir: &str, relative: &str) -> String {
    let joined = Path::new(base_dir).join(relative);
    normalize_path(&joined.to_string_lossy())
}
