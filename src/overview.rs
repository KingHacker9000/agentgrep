use anyhow::Result;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;

use crate::index::RepoIndex;
use crate::types::{ConnectedPair, OverviewReport, OverviewSymbol, PackageGroup, SymbolKind};

const DEFAULT_TYPE_LIMIT: usize = 20;
const DEFAULT_VOCAB_LIMIT: usize = 20;
const MAX_CONNECTED: usize = 8;
const MAX_PACKAGES: usize = 8;
const MAX_ENTRY_POINTS: usize = 5;

/// File names treated as entry points.
const ENTRY_NAMES: &[&str] = &[
    "main.rs", "lib.rs", "main.py", "__main__.py", "app.py",
    "index.ts", "index.js", "main.ts", "main.go",
    "index.tsx", "server.ts", "server.js", "app.ts", "app.js",
];

/// Parsed set of requested sections. Empty means "all".
struct Sections(HashSet<String>);

impl Sections {
    fn from_args(only: &[String]) -> Self {
        Self(only.iter().map(|s| s.to_ascii_lowercase()).collect())
    }

    fn wants(&self, section: &str) -> bool {
        self.0.is_empty() || self.0.contains(section)
    }

    /// Vocab is derived from types/functions — include types/fns in computation
    /// if vocab was explicitly requested.
    fn wants_types_for_vocab(&self) -> bool {
        !self.0.is_empty() && self.0.contains("vocab") && !self.0.contains("types")
    }

    fn wants_fns_for_vocab(&self) -> bool {
        !self.0.is_empty() && self.0.contains("vocab") && !self.0.contains("functions")
    }
}

pub fn build_report(
    index: &RepoIndex,
    full: bool,
    min_refs: usize,
    only: &[String],
) -> Result<OverviewReport> {
    let ref_counts = build_ref_counts(index);
    let sections = Sections::from_args(only);

    let type_kinds = [
        SymbolKind::Struct,
        SymbolKind::Enum,
        SymbolKind::Trait,
        SymbolKind::TypeAlias,
    ];
    let fn_kinds = [SymbolKind::Function];

    let type_limit = if full { usize::MAX } else { DEFAULT_TYPE_LIMIT };
    let fn_limit = if full { usize::MAX } else { DEFAULT_TYPE_LIMIT };

    // Compute types if requested directly or needed for vocab derivation.
    let key_types = if sections.wants("types") || sections.wants_types_for_vocab() {
        ranked_symbols(index, &ref_counts, &type_kinds, type_limit, min_refs)
    } else {
        vec![]
    };

    // Compute functions if: --full set, or explicitly requested, or needed for vocab.
    let key_functions = if sections.wants("functions") || sections.wants_fns_for_vocab() {
        ranked_symbols(index, &ref_counts, &fn_kinds, fn_limit, min_refs)
    } else if full && sections.wants("functions") {
        ranked_symbols(index, &ref_counts, &fn_kinds, usize::MAX, min_refs)
    } else if full && sections.0.is_empty() {
        // --full with no --only: include all functions.
        ranked_symbols(index, &ref_counts, &fn_kinds, usize::MAX, min_refs)
    } else {
        vec![]
    };

    // Vocabulary: type names first, then function names, capped.
    let vocabulary = if sections.wants("vocab") {
        key_types
            .iter()
            .chain(key_functions.iter())
            .map(|s| s.name.clone())
            .take(DEFAULT_VOCAB_LIMIT)
            .collect()
    } else {
        vec![]
    };

    let entry_points = if sections.wants("entries") {
        detect_entry_points(index)
    } else {
        vec![]
    };

    let packages = if sections.wants("packages") {
        detect_packages(index)
    } else {
        vec![]
    };

    let most_connected = if sections.wants("connected") {
        detect_most_connected(index)
    } else {
        vec![]
    };

    // Omit key_types/key_functions from the report if they were only computed for vocab.
    let report_types = if sections.wants("types") {
        key_types.clone()
    } else {
        vec![]
    };
    let report_functions = if sections.wants("functions") {
        key_functions.clone()
    } else {
        vec![]
    };

    Ok(OverviewReport {
        repo_root: index.repo_root.clone(),
        languages: detect_languages(index),
        file_count: index.stats.file_count,
        symbol_count: index.stats.symbol_count,
        entry_points,
        packages,
        key_types: report_types,
        key_functions: report_functions,
        most_connected,
        vocabulary,
    })
}

/// Count how many `symbol_references` records name each symbol.
fn build_ref_counts(index: &RepoIndex) -> HashMap<String, usize> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for r in &index.symbol_references {
        *counts.entry(r.symbol_name.clone()).or_insert(0) += 1;
    }
    counts
}

/// Return public symbols of the given kinds, filtered by min_refs, ranked by ref_count desc.
fn ranked_symbols(
    index: &RepoIndex,
    ref_counts: &HashMap<String, usize>,
    kinds: &[SymbolKind],
    limit: usize,
    min_refs: usize,
) -> Vec<OverviewSymbol> {
    use crate::types::Visibility;

    let mut candidates: Vec<OverviewSymbol> = index
        .symbols
        .iter()
        .filter(|s| {
            kinds.contains(&s.kind)
                && matches!(s.visibility, Visibility::Public)
                && !s.name.is_empty()
        })
        .filter_map(|s| {
            let ref_count = ref_counts.get(&s.name).copied().unwrap_or(0);
            if ref_count >= min_refs {
                Some(OverviewSymbol {
                    name: s.name.clone(),
                    kind: s.kind.to_string(),
                    file: s.file_path.clone(),
                    line: s.line_number,
                    ref_count,
                })
            } else {
                None
            }
        })
        .collect();

    candidates.sort_by(|a, b| {
        b.ref_count
            .cmp(&a.ref_count)
            .then_with(|| a.name.cmp(&b.name))
    });

    // Deduplicate by name — keep highest-ref entry.
    let mut seen = HashSet::new();
    candidates.retain(|c| seen.insert(c.name.clone()));

    candidates.truncate(limit);
    candidates
}

fn detect_languages(index: &RepoIndex) -> Vec<String> {
    let mut ext_counts: BTreeMap<&str, usize> = BTreeMap::new();
    for file in &index.files {
        let ext = match file.path.rsplit_once('.').map(|(_, e)| e) {
            Some("rs") => "rust",
            Some("py") => "python",
            Some("ts") | Some("tsx") => "typescript",
            Some("js") | Some("jsx") | Some("mjs") | Some("cjs") => "javascript",
            Some("go") => "go",
            _ => continue,
        };
        *ext_counts.entry(ext).or_insert(0) += 1;
    }
    let mut langs: Vec<(&str, usize)> = ext_counts.into_iter().collect();
    langs.sort_by(|a, b| b.1.cmp(&a.1));
    langs.into_iter().take(3).map(|(l, _)| l.to_string()).collect()
}

fn detect_entry_points(index: &RepoIndex) -> Vec<String> {
    use crate::index::FileRole;
    let mut entries: Vec<String> = index
        .files
        .iter()
        .filter(|f| matches!(f.role, FileRole::Source))
        .filter(|f| {
            let name = f.path.rsplit('/').next().unwrap_or(&f.path);
            let name = name.rsplit('\\').next().unwrap_or(name);
            ENTRY_NAMES.contains(&name)
        })
        .map(|f| f.path.clone())
        .collect();
    entries.sort_by(|a, b| a.len().cmp(&b.len()).then(a.cmp(b)));
    entries.truncate(MAX_ENTRY_POINTS);
    entries
}

fn detect_packages(index: &RepoIndex) -> Vec<PackageGroup> {
    use crate::index::FileRole;
    let mut groups: BTreeMap<String, usize> = BTreeMap::new();
    for file in index.files.iter().filter(|f| matches!(f.role, FileRole::Source)) {
        *groups.entry(top_level_prefix(&file.path)).or_insert(0) += 1;
    }
    let mut groups: Vec<PackageGroup> = groups
        .into_iter()
        .map(|(prefix, source_file_count)| PackageGroup { prefix, source_file_count })
        .collect();
    groups.sort_by(|a, b| {
        b.source_file_count.cmp(&a.source_file_count).then(a.prefix.cmp(&b.prefix))
    });
    groups.truncate(MAX_PACKAGES);
    groups
}

/// `src/foo/bar.rs` → `src/`, `crates/printer/src/color.rs` → `crates/printer/`
fn top_level_prefix(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    let parts: Vec<&str> = normalized.splitn(4, '/').collect();
    if let Some(&first) = parts.first() {
        if matches!(first, "crates" | "packages" | "apps" | "modules" | "libs") {
            if parts.len() >= 2 {
                return format!("{}/{}/", parts[0], parts[1]);
            }
        }
        if !first.is_empty() {
            return format!("{}/", first);
        }
    }
    normalized.split('/').next().map(|s| format!("{s}/")).unwrap_or_else(|| "./".to_string())
}

fn detect_most_connected(index: &RepoIndex) -> Vec<ConnectedPair> {
    let mut pair_counts: HashMap<(String, String), usize> = HashMap::new();
    for edge in &index.edges {
        let (from, to) = if edge.from <= edge.to {
            (edge.from.clone(), edge.to.clone())
        } else {
            (edge.to.clone(), edge.from.clone())
        };
        if from != to {
            *pair_counts.entry((from, to)).or_insert(0) += 1;
        }
    }
    let mut pairs: Vec<ConnectedPair> = pair_counts
        .into_iter()
        .filter(|(_, count)| *count >= 2)
        .map(|((from, to), edge_count)| ConnectedPair { from, to, edge_count })
        .collect();
    pairs.sort_by(|a, b| b.edge_count.cmp(&a.edge_count).then(a.from.cmp(&b.from)));
    pairs.truncate(MAX_CONNECTED);
    pairs
}

pub fn write_report(report: &OverviewReport, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(report)?);
        return Ok(());
    }

    let lang_str = report.languages.join(" · ");
    println!(
        "Overview: {}  ({} · {} files · {} symbols)",
        report.repo_root, lang_str, report.file_count, report.symbol_count
    );

    if !report.entry_points.is_empty() {
        println!();
        println!("Entry points:");
        for ep in &report.entry_points {
            println!("  {ep}");
        }
    }

    if !report.packages.is_empty() {
        println!();
        println!("Packages:");
        for pkg in &report.packages {
            println!("  {:40} ({} source files)", pkg.prefix, pkg.source_file_count);
        }
    }

    if !report.key_types.is_empty() {
        println!();
        println!("Key types ({}):", report.key_types.len());
        for s in &report.key_types {
            println!(
                "  {:<30}  {:12}  {}:{}{}",
                s.name, s.kind, s.file, s.line,
                if s.ref_count > 0 { format!("  [ref:{}]", s.ref_count) } else { String::new() }
            );
        }
    }

    if !report.key_functions.is_empty() {
        println!();
        println!("Key functions ({}):", report.key_functions.len());
        for s in &report.key_functions {
            println!(
                "  {:<30}  {:12}  {}:{}{}",
                s.name, s.kind, s.file, s.line,
                if s.ref_count > 0 { format!("  [ref:{}]", s.ref_count) } else { String::new() }
            );
        }
    }

    if !report.most_connected.is_empty() {
        println!();
        println!("Most connected:");
        for pair in &report.most_connected {
            println!("  {}  ↔  {}  ({} edges)", pair.from, pair.to, pair.edge_count);
        }
    }

    if !report.vocabulary.is_empty() {
        println!();
        println!("Vocabulary: {}", report.vocabulary.join(", "));
    }

    Ok(())
}
