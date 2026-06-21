use anyhow::{anyhow, Result};
use std::collections::BTreeMap;
use std::path::Path;

use crate::index::{self, IndexedEdge, IndexedFile};
use crate::repo::{display_path, RepoInfo};
use crate::text::shorten_snippet;
use crate::types::{ConnectionCounts, IndexedSymbol, MapEdge, MapReport};

const MAP_EDGE_DISPLAY_LIMIT: usize = 5;
const MAP_SYMBOL_DISPLAY_LIMIT: usize = 5;
const EDGE_TYPE_PRIORITY: [&str; 6] = [
    "declares_module",
    "imports",
    "references",
    "likely_test_for",
    "configures",
    "same_area",
];

pub fn build_report(repo: &RepoInfo, input_path: &str) -> Result<MapReport> {
    let loaded = index::load(repo)?;
    let resolved_path = resolve_requested_path(&repo.root, input_path);

    let Some(index) = loaded.index else {
        return Ok(build_missing_report(
            &loaded.index_path,
            loaded.state.to_string(),
            repo,
            &resolved_path,
        ));
    };

    let file = index
        .files
        .iter()
        .find(|file| file.path == resolved_path)
        .ok_or_else(|| anyhow!("File not found in index: {}", resolved_path))?;

    let outgoing_all: Vec<&IndexedEdge> = index
        .edges
        .iter()
        .filter(|edge| edge.from == resolved_path)
        .collect();
    let incoming_all: Vec<&IndexedEdge> = index
        .edges
        .iter()
        .filter(|edge| edge.to == resolved_path)
        .collect();

    let outgoing_display = ordered_edges(&outgoing_all);
    let incoming_display = ordered_edges(&incoming_all);
    let symbols = ordered_symbols(
        index
            .symbols
            .iter()
            .filter(|symbol| symbol.file_path == resolved_path)
            .collect::<Vec<_>>(),
    );

    let connection_counts = ConnectionCounts {
        outgoing_total: outgoing_all.len(),
        incoming_total: incoming_all.len(),
        outgoing_by_type: count_edges_by_type(&outgoing_all),
        incoming_by_type: count_edges_by_type(&incoming_all),
    };

    Ok(MapReport {
        path: resolved_path,
        role: file.role.to_string(),
        index_status: loaded.state.to_string(),
        index_path: display_path(&loaded.index_path),
        repo_rev: repo.rev.clone(),
        size_bytes: file.size_bytes,
        modified_unix: file.modified_unix,
        content_hash: file.content_hash.clone(),
        symbols: render_symbols(&symbols),
        outgoing_edges: render_edges(&outgoing_display),
        incoming_edges: render_edges(&incoming_display),
        connection_counts,
        next_actions: build_next_actions(repo, file, &loaded.state.to_string()),
    })
}

fn build_missing_report(
    index_path: &Path,
    status: String,
    repo: &RepoInfo,
    path: &str,
) -> MapReport {
    MapReport {
        path: path.to_string(),
        role: "other".to_string(),
        index_status: status,
        index_path: display_path(index_path),
        repo_rev: repo.rev.clone(),
        size_bytes: None,
        modified_unix: None,
        content_hash: None,
        symbols: Vec::new(),
        outgoing_edges: Vec::new(),
        incoming_edges: Vec::new(),
        connection_counts: ConnectionCounts {
            outgoing_total: 0,
            incoming_total: 0,
            outgoing_by_type: BTreeMap::new(),
            incoming_by_type: BTreeMap::new(),
        },
        next_actions: vec![
            format!("open {}", path),
            "agentgrep index".to_string(),
            "agentgrep index --status".to_string(),
        ],
    }
}

pub fn write_report(report: &MapReport, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(report)?);
        return Ok(());
    }

    println!("File card:");
    println!("- path: {}", report.path);
    println!("- role: {}", report.role);
    println!("- index status: {}", report.index_status);
    if let Some(size_bytes) = report.size_bytes {
        println!("- size: {} bytes", size_bytes);
    }
    if let Some(modified_unix) = report.modified_unix {
        println!("- modified: {}", modified_unix);
    }
    if let Some(content_hash) = &report.content_hash {
        println!("- hash: {}", content_hash);
    }
    if let Some(repo_rev) = &report.repo_rev {
        println!("- repo rev: {}", repo_rev);
    }
    println!("- index path: {}", report.index_path);

    if report.symbols.is_empty() {
        println!("Symbols: none indexed");
    } else {
        println!("Symbols ({} total):", report.symbols.len());
        render_symbol_section(&report.symbols);
    }

    println!(
        "Outgoing ({} total):",
        report.connection_counts.outgoing_total
    );
    render_edge_section(
        &report.outgoing_edges,
        &report.connection_counts.outgoing_by_type,
    );

    println!(
        "Incoming ({} total):",
        report.connection_counts.incoming_total
    );
    render_edge_section(
        &report.incoming_edges,
        &report.connection_counts.incoming_by_type,
    );

    println!("Next:");
    for action in &report.next_actions {
        println!("- {action}");
    }

    if report.index_status == "missing" {
        println!();
        println!("Index missing. Run `agentgrep index` first.");
    } else if report.index_status == "unverifiable" {
        println!();
        println!("Index unverifiable because repo revision is unavailable.");
    }

    Ok(())
}

fn render_edge_section(edges: &[MapEdge], counts: &BTreeMap<String, usize>) {
    if edges.is_empty() {
        println!("- none");
    } else {
        for edge in edges.iter().take(MAP_EDGE_DISPLAY_LIMIT) {
            println!(
                "- {} -> {} [{}] {}",
                edge.from, edge.to, edge.edge_type, edge.reason
            );
        }
        if edges.len() > MAP_EDGE_DISPLAY_LIMIT {
            println!(
                "- ... showing {} of {}",
                MAP_EDGE_DISPLAY_LIMIT,
                edges.len()
            );
        }
    }

    if !counts.is_empty() {
        println!(
            "  counts: {}",
            ordered_count_items(counts)
                .into_iter()
                .map(|(edge_type, count)| format!("{edge_type}:{count}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
}

fn render_symbol_section(symbols: &[IndexedSymbol]) {
    for symbol in symbols.iter().take(MAP_SYMBOL_DISPLAY_LIMIT) {
        let mut details = format!(
            "{} [{} {}] line {}",
            symbol.name, symbol.kind, symbol.visibility, symbol.line_number
        );
        if let Some(signature) = &symbol.signature {
            details.push_str(": ");
            details.push_str(signature);
        }
        println!("- {details}");
    }
    if symbols.len() > MAP_SYMBOL_DISPLAY_LIMIT {
        println!(
            "- ... showing {} of {}",
            MAP_SYMBOL_DISPLAY_LIMIT,
            symbols.len()
        );
    }
}

pub(crate) fn ordered_edges<'a>(edges: &'a [&'a IndexedEdge]) -> Vec<&'a IndexedEdge> {
    let mut ordered = edges.to_vec();
    ordered.sort_by(|left, right| {
        edge_type_rank(&left.edge_type)
            .cmp(&edge_type_rank(&right.edge_type))
            .then_with(|| left.edge_type.cmp(&right.edge_type))
            .then_with(|| left.from.cmp(&right.from))
            .then_with(|| left.to.cmp(&right.to))
            .then_with(|| left.reason.cmp(&right.reason))
    });
    ordered
}

fn ordered_symbols<'a>(symbols: Vec<&'a IndexedSymbol>) -> Vec<&'a IndexedSymbol> {
    let mut ordered = symbols;
    ordered.sort_by(|left, right| {
        left.line_number
            .cmp(&right.line_number)
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.kind.cmp(&right.kind))
    });
    ordered
}

fn count_edges_by_type(edges: &[&IndexedEdge]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for edge in edges {
        *counts.entry(edge.edge_type.clone()).or_insert(0) += 1;
    }
    counts
}

fn ordered_count_items(counts: &BTreeMap<String, usize>) -> Vec<(&String, &usize)> {
    let mut items = counts.iter().collect::<Vec<_>>();
    items.sort_by(|left, right| {
        edge_type_rank(left.0)
            .cmp(&edge_type_rank(right.0))
            .then_with(|| left.0.cmp(right.0))
    });
    items
}

fn render_edges(edges: &[&IndexedEdge]) -> Vec<MapEdge> {
    edges
        .iter()
        .take(MAP_EDGE_DISPLAY_LIMIT)
        .map(|edge| MapEdge {
            edge_type: edge.edge_type.clone(),
            from: edge.from.clone(),
            to: edge.to.clone(),
            confidence: edge.confidence.to_string(),
            reason: edge.reason.clone(),
        })
        .collect()
}

fn render_symbols(symbols: &[&IndexedSymbol]) -> Vec<IndexedSymbol> {
    symbols
        .iter()
        .map(|symbol| IndexedSymbol {
            name: symbol.name.clone(),
            kind: symbol.kind.clone(),
            file_path: symbol.file_path.clone(),
            line_number: symbol.line_number,
            visibility: symbol.visibility.clone(),
            signature: symbol
                .signature
                .as_ref()
                .map(|signature| shorten_snippet(signature, 120)),
            end_line: symbol.end_line,
            parent_class: symbol.parent_class.clone(),
        })
        .collect()
}

fn edge_type_rank(edge_type: &str) -> usize {
    EDGE_TYPE_PRIORITY
        .iter()
        .position(|candidate| candidate == &edge_type)
        .unwrap_or(EDGE_TYPE_PRIORITY.len())
}

pub(crate) fn build_next_actions(
    repo: &RepoInfo,
    file: &IndexedFile,
    index_status: &str,
) -> Vec<String> {
    let mut actions = Vec::new();
    actions.push(format!("open {}", file.path));

    let stem = file_stem_like(&file.path);
    if !stem.is_empty() {
        actions.push(format!("agentgrep find \"{}\"", stem));
    }

    if index_status != "fresh" {
        actions.push("agentgrep index --status".to_string());
    }

    if repo.rev.is_none() {
        actions.push("agentgrep index".to_string());
    }

    actions
}

pub(crate) fn resolve_requested_path(repo_root: &Path, input: &str) -> String {
    let requested = Path::new(input);
    let absolute = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        repo_root.join(requested)
    };

    absolute
        .strip_prefix(repo_root)
        .map(display_path)
        .unwrap_or_else(|_| display_path(&absolute))
}

fn file_stem_like(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(|stem| stem.replace('_', " "))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::{
        EdgeConfidence, FileRole, IndexState, IndexStats, IndexedFile, RepoIndex,
        INDEX_SCHEMA_VERSION,
    };
    use std::path::PathBuf;

    #[test]
    fn resolves_relative_paths_against_repo_root() {
        let repo_root = PathBuf::from("C:/repo");
        assert_eq!(
            resolve_requested_path(&repo_root, "src/search.rs"),
            "src/search.rs"
        );
    }

    #[test]
    fn build_report_uses_file_entry_and_edges() {
        let repo = RepoInfo {
            root: PathBuf::from("C:/repo"),
            rev: Some("abc".to_string()),
            git_dir: None,
        };
        let loaded = index::LoadedIndex {
            index_path: PathBuf::from("C:/repo/.agentgrep/index.json"),
            state: IndexState::Fresh,
            index: Some(RepoIndex {
                schema_version: INDEX_SCHEMA_VERSION,
                repo_root: "C:/repo".to_string(),
                repo_rev: Some("abc".to_string()),
                indexed_at_unix: 1,
                files: vec![IndexedFile {
                    path: "src/search.rs".to_string(),
                    role: FileRole::Source,
                    size_bytes: Some(10),
                    modified_unix: Some(20),
                    content_hash: Some("deadbeef".to_string()),
                    ..Default::default()
                }],
                symbols: vec![],
                symbol_references: vec![],
                edges: vec![crate::index::IndexedEdge {
                    edge_type: "same_area".to_string(),
                    from: "src/search.rs".to_string(),
                    to: "src/index.rs".to_string(),
                    confidence: EdgeConfidence::Extracted,
                    reason: "shared source area src".to_string(),
                }],
                stats: IndexStats {
                    file_count: 1,
                    role_counts: BTreeMap::from([(FileRole::Source, 1)]),
                    symbol_count: 0,
                    symbol_kind_counts: BTreeMap::new(),
                    symbol_reference_count: 0,
                    connection_count: 1,
                    ..Default::default()
                },
            }),
        };
        let report = build_report_from_loaded(&repo, &loaded, "src/search.rs").unwrap();
        assert_eq!(report.path, "src/search.rs");
        assert_eq!(report.role, "source");
        assert_eq!(report.outgoing_edges.len(), 1);
        assert_eq!(report.connection_counts.outgoing_total, 1);
    }

    #[test]
    fn missing_index_report_contains_actions() {
        let repo = RepoInfo {
            root: PathBuf::from("C:/repo"),
            rev: None,
            git_dir: None,
        };
        let report = build_missing_report(
            Path::new("C:/repo/.agentgrep/index.json"),
            "missing".to_string(),
            &repo,
            "src/search.rs",
        );
        assert_eq!(report.index_status, "missing");
        assert!(report
            .next_actions
            .iter()
            .any(|action| action == "agentgrep index"));
    }

    #[test]
    fn prioritizes_import_edges_in_map_output() {
        let repo = RepoInfo {
            root: PathBuf::from("C:/repo"),
            rev: Some("abc".to_string()),
            git_dir: None,
        };
        let loaded = index::LoadedIndex {
            index_path: PathBuf::from("C:/repo/.agentgrep/index.json"),
            state: IndexState::Fresh,
            index: Some(RepoIndex {
                schema_version: INDEX_SCHEMA_VERSION,
                repo_root: "C:/repo".to_string(),
                repo_rev: Some("abc".to_string()),
                indexed_at_unix: 1,
                files: vec![IndexedFile {
                    path: "src/search.rs".to_string(),
                    role: FileRole::Source,
                    size_bytes: Some(10),
                    modified_unix: Some(20),
                    content_hash: Some("deadbeef".to_string()),
                    ..Default::default()
                }],
                symbols: vec![],
                symbol_references: vec![],
                edges: vec![
                    crate::index::IndexedEdge {
                        edge_type: "same_area".to_string(),
                        from: "src/search.rs".to_string(),
                        to: "src/types.rs".to_string(),
                        confidence: EdgeConfidence::Extracted,
                        reason: "shared source area src".to_string(),
                    },
                    crate::index::IndexedEdge {
                        edge_type: "imports".to_string(),
                        from: "src/search.rs".to_string(),
                        to: "src/text.rs".to_string(),
                        confidence: EdgeConfidence::Extracted,
                        reason: "imports crate::text".to_string(),
                    },
                ],
                stats: IndexStats {
                    file_count: 1,
                    role_counts: BTreeMap::from([(FileRole::Source, 1)]),
                    symbol_count: 0,
                    symbol_kind_counts: BTreeMap::new(),
                    symbol_reference_count: 0,
                    connection_count: 2,
                    ..Default::default()
                },
            }),
        };
        let report = build_report_from_loaded(&repo, &loaded, "src/search.rs").unwrap();
        assert_eq!(report.outgoing_edges.first().unwrap().edge_type, "imports");
        assert_eq!(
            report.connection_counts.outgoing_by_type.get("imports"),
            Some(&1)
        );
    }

    #[test]
    fn map_report_includes_symbols_and_json() {
        let repo = RepoInfo {
            root: PathBuf::from("C:/repo"),
            rev: Some("abc".to_string()),
            git_dir: None,
        };
        let loaded = index::LoadedIndex {
            index_path: PathBuf::from("C:/repo/.agentgrep/index.json"),
            state: IndexState::Fresh,
            index: Some(RepoIndex {
                schema_version: INDEX_SCHEMA_VERSION,
                repo_root: "C:/repo".to_string(),
                repo_rev: Some("abc".to_string()),
                indexed_at_unix: 1,
                files: vec![IndexedFile {
                    path: "src/search.rs".to_string(),
                    role: FileRole::Source,
                    size_bytes: Some(10),
                    modified_unix: Some(20),
                    content_hash: Some("deadbeef".to_string()),
                    ..Default::default()
                }],
                symbols: vec![crate::types::IndexedSymbol {
                    name: "run".to_string(),
                    kind: crate::types::SymbolKind::Function,
                    file_path: "src/search.rs".to_string(),
                    line_number: 12,
                    visibility: crate::types::Visibility::Public,
                    signature: Some("pub fn run()".to_string()),
                    end_line: None,

            parent_class: None,                }],
                symbol_references: vec![],
                edges: vec![],
                stats: IndexStats {
                    file_count: 1,
                    role_counts: BTreeMap::from([(FileRole::Source, 1)]),
                    symbol_count: 1,
                    symbol_kind_counts: BTreeMap::from([(crate::types::SymbolKind::Function, 1)]),
                    symbol_reference_count: 0,
                    connection_count: 0,
                    ..Default::default()
                },
            }),
        };
        let report = build_report_from_loaded(&repo, &loaded, "src/search.rs").unwrap();
        assert_eq!(report.symbols.len(), 1);
        assert_eq!(report.symbols[0].name, "run");

        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["symbols"].as_array().unwrap().len(), 1);
        assert_eq!(json["symbols"][0]["name"], "run");
    }

    fn build_report_from_loaded(
        repo: &RepoInfo,
        loaded: &index::LoadedIndex,
        input_path: &str,
    ) -> Result<MapReport> {
        let resolved_path = resolve_requested_path(&repo.root, input_path);
        let index = loaded.index.as_ref().unwrap();
        let file = index
            .files
            .iter()
            .find(|file| file.path == resolved_path)
            .unwrap();
        let outgoing_all: Vec<&IndexedEdge> = index
            .edges
            .iter()
            .filter(|edge| edge.from == resolved_path)
            .collect();
        let incoming_all: Vec<&IndexedEdge> = index
            .edges
            .iter()
            .filter(|edge| edge.to == resolved_path)
            .collect();
        let symbols: Vec<&crate::types::IndexedSymbol> = index
            .symbols
            .iter()
            .filter(|symbol| symbol.file_path == resolved_path)
            .collect();
        Ok(MapReport {
            path: resolved_path,
            role: file.role.to_string(),
            index_status: loaded.state.to_string(),
            index_path: display_path(&loaded.index_path),
            repo_rev: repo.rev.clone(),
            size_bytes: file.size_bytes,
            modified_unix: file.modified_unix,
            content_hash: file.content_hash.clone(),
            symbols: render_symbols(&ordered_symbols(symbols)),
            outgoing_edges: render_edges(&ordered_edges(&outgoing_all)),
            incoming_edges: render_edges(&ordered_edges(&incoming_all)),
            connection_counts: ConnectionCounts {
                outgoing_total: outgoing_all.len(),
                incoming_total: incoming_all.len(),
                outgoing_by_type: count_edges_by_type(&outgoing_all),
                incoming_by_type: count_edges_by_type(&incoming_all),
            },
            next_actions: build_next_actions(repo, file, &loaded.state.to_string()),
        })
    }
}
