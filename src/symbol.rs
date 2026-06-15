use anyhow::{anyhow, Result};
use std::collections::BTreeMap;

use crate::index::{self, FileRole, IndexedEdge, IndexedFile};
use crate::map;
use crate::repo::RepoInfo;
use crate::types::{IndexedSymbol, MapEdge, SymbolMatch, SymbolMatchMode, SymbolReport};

const SYMBOL_MATCH_LIMIT: usize = 5;
const SYMBOL_EDGE_DISPLAY_LIMIT: usize = 3;

pub fn build_report(repo: &RepoInfo, query: &str) -> Result<SymbolReport> {
    let loaded = index::load(repo)?;
    build_report_from_loaded(repo, &loaded, query)
}

fn build_report_from_loaded(
    repo: &RepoInfo,
    loaded: &index::LoadedIndex,
    query: &str,
) -> Result<SymbolReport> {
    let query = query.trim().to_string();
    if query.is_empty() {
        return Err(anyhow!("symbol name must not be empty"));
    }

    let Some(index) = loaded.index.as_ref() else {
        return Ok(build_missing_report(loaded.state.to_string(), &query));
    };

    let (match_mode, candidates) = match_symbols(&index.symbols, &query);
    let mut file_contexts: BTreeMap<String, SymbolFileContext> = BTreeMap::new();
    let mut matches = Vec::new();

    for symbol in candidates.into_iter().take(SYMBOL_MATCH_LIMIT) {
        let context = build_symbol_file_context(
            repo,
            &loaded.state.to_string(),
            index,
            symbol,
            &mut file_contexts,
            &query,
        );
        matches.push(context);
    }

    let next_actions =
        build_symbol_next_actions(&query, &matches, repo, loaded.state.to_string().as_str());

    Ok(SymbolReport {
        query,
        index_status: loaded.state.to_string(),
        match_mode,
        matches,
        next_actions,
    })
}

fn build_missing_report(status: String, query: &str) -> SymbolReport {
    SymbolReport {
        query: query.to_string(),
        index_status: status,
        match_mode: SymbolMatchMode::None,
        matches: Vec::new(),
        next_actions: vec![
            format!("agentgrep find \"{}\"", query),
            "agentgrep index".to_string(),
            "agentgrep index --status".to_string(),
        ],
    }
}

pub fn write_report(report: &SymbolReport, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(report)?);
        return Ok(());
    }

    println!("Symbol query: {}", report.query);
    println!("- index status: {}", report.index_status);
    println!("- match mode: {}", report.match_mode);

    if report.matches.is_empty() {
        println!("Matches: none");
    } else {
        println!("Matches ({} total):", report.matches.len());
        for item in &report.matches {
            render_match(item);
        }
    }

    if report.matches.is_empty() {
        println!("Next:");
        for action in &report.next_actions {
            println!("- {action}");
        }
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

fn render_match(item: &SymbolMatch) {
    println!(
        "- {} [{} {}] {}:{}",
        item.symbol.name,
        item.symbol.kind,
        item.symbol.visibility,
        item.symbol.file_path,
        item.symbol.line_number
    );
    if let Some(signature) = &item.symbol.signature {
        println!("  signature: {}", signature);
    }
    println!("  role: {}", item.file_role);
    render_edges("Outgoing", &item.outgoing_edges);
    render_edges("Incoming", &item.incoming_edges);
    println!("  next:");
    for action in &item.next_actions {
        println!("  - {action}");
    }
}

fn render_edges(label: &str, edges: &[MapEdge]) {
    if edges.is_empty() {
        println!("  {label}: none");
        return;
    }

    println!("  {label} ({} total):", edges.len());
    for edge in edges.iter().take(SYMBOL_EDGE_DISPLAY_LIMIT) {
        println!(
            "  - {} -> {} [{}] {}",
            edge.from, edge.to, edge.edge_type, edge.reason
        );
    }
    if edges.len() > SYMBOL_EDGE_DISPLAY_LIMIT {
        println!(
            "  - ... showing {} of {}",
            SYMBOL_EDGE_DISPLAY_LIMIT,
            edges.len()
        );
    }
}

fn match_symbols<'a>(
    symbols: &'a [IndexedSymbol],
    query: &str,
) -> (SymbolMatchMode, Vec<&'a IndexedSymbol>) {
    let mut exact = symbols
        .iter()
        .filter(|symbol| symbol.name == query)
        .collect::<Vec<_>>();
    sort_symbols(&mut exact);
    if !exact.is_empty() {
        return (SymbolMatchMode::Exact, exact);
    }

    let mut insensitive = symbols
        .iter()
        .filter(|symbol| symbol.name.eq_ignore_ascii_case(query))
        .collect::<Vec<_>>();
    sort_symbols(&mut insensitive);
    if !insensitive.is_empty() {
        return (SymbolMatchMode::CaseInsensitive, insensitive);
    }

    let query_lower = query.to_lowercase();
    let mut substring = symbols
        .iter()
        .filter(|symbol| symbol.name.to_lowercase().contains(&query_lower))
        .collect::<Vec<_>>();
    sort_symbols(&mut substring);
    if !substring.is_empty() {
        return (SymbolMatchMode::Substring, substring);
    }

    (SymbolMatchMode::None, Vec::new())
}

fn sort_symbols(symbols: &mut Vec<&IndexedSymbol>) {
    symbols.sort_by(|left, right| {
        left.file_path
            .cmp(&right.file_path)
            .then_with(|| left.line_number.cmp(&right.line_number))
            .then_with(|| left.name.cmp(&right.name))
    });
}

fn build_symbol_file_context(
    repo: &RepoInfo,
    index_status: &str,
    index: &index::RepoIndex,
    symbol: &IndexedSymbol,
    cache: &mut BTreeMap<String, SymbolFileContext>,
    query: &str,
) -> SymbolMatch {
    let context = if let Some(context) = cache.get(&symbol.file_path) {
        context.clone()
    } else {
        let context = build_file_context(repo, index_status, index, &symbol.file_path, query);
        cache.insert(symbol.file_path.clone(), context.clone());
        context
    };

    SymbolMatch {
        symbol: symbol.clone(),
        file_role: context.file_role,
        outgoing_edges: context.outgoing_edges,
        incoming_edges: context.incoming_edges,
        next_actions: context.next_actions,
    }
}

#[derive(Clone)]
struct SymbolFileContext {
    file_role: String,
    outgoing_edges: Vec<MapEdge>,
    incoming_edges: Vec<MapEdge>,
    next_actions: Vec<String>,
}

fn build_file_context(
    repo: &RepoInfo,
    index_status: &str,
    index: &index::RepoIndex,
    file_path: &str,
    query: &str,
) -> SymbolFileContext {
    let file = index
        .files
        .iter()
        .find(|item| item.path == file_path)
        .cloned()
        .unwrap_or_else(|| IndexedFile {
            path: file_path.to_string(),
            role: FileRole::Other,
            size_bytes: None,
            modified_unix: None,
            content_hash: None,
        });

    let outgoing_all: Vec<&IndexedEdge> = index
        .edges
        .iter()
        .filter(|edge| edge.from == file_path)
        .collect();
    let incoming_all: Vec<&IndexedEdge> = index
        .edges
        .iter()
        .filter(|edge| edge.to == file_path)
        .collect();

    SymbolFileContext {
        file_role: file.role.to_string(),
        outgoing_edges: edge_list(&map::ordered_edges(&outgoing_all)),
        incoming_edges: edge_list(&map::ordered_edges(&incoming_all)),
        next_actions: build_file_next_actions(repo, &file, index_status, query),
    }
}

fn build_file_next_actions(
    repo: &RepoInfo,
    file: &IndexedFile,
    index_status: &str,
    query: &str,
) -> Vec<String> {
    let mut actions = vec![format!("open {}", file.path)];
    push_unique_action(&mut actions, format!("agentgrep map {}", file.path));
    push_unique_action(&mut actions, format!("agentgrep find \"{}\"", query));

    if index_status != "fresh" {
        push_unique_action(&mut actions, "agentgrep index --status".to_string());
    }

    if repo.rev.is_none() {
        push_unique_action(&mut actions, "agentgrep index".to_string());
    }

    actions
}

fn push_unique_action(actions: &mut Vec<String>, action: String) {
    if !actions.iter().any(|existing| existing == &action) {
        actions.push(action);
    }
}

fn edge_list(edges: &[&IndexedEdge]) -> Vec<MapEdge> {
    edges
        .iter()
        .take(SYMBOL_EDGE_DISPLAY_LIMIT)
        .map(|edge| MapEdge {
            edge_type: edge.edge_type.clone(),
            from: edge.from.clone(),
            to: edge.to.clone(),
            confidence: edge.confidence.to_string(),
            reason: edge.reason.clone(),
        })
        .collect()
}

fn build_symbol_next_actions(
    query: &str,
    matches: &[SymbolMatch],
    repo: &RepoInfo,
    index_status: &str,
) -> Vec<String> {
    let mut actions = Vec::new();

    if let Some(first) = matches.first() {
        actions.push(format!("open {}", first.symbol.file_path));
        actions.push(format!("agentgrep map {}", first.symbol.file_path));
        actions.push(format!("agentgrep find \"{}\"", query));
    } else {
        actions.push(format!("agentgrep find \"{}\"", query));
    }

    if index_status != "fresh" {
        actions.push("agentgrep index --status".to_string());
    }

    if repo.rev.is_none() {
        actions.push("agentgrep index".to_string());
    }

    actions
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::{EdgeConfidence, FileRole, IndexState, IndexStats, IndexedEdge, RepoIndex};
    use std::path::PathBuf;

    fn repo() -> RepoInfo {
        RepoInfo {
            root: PathBuf::from("C:/repo"),
            rev: Some("abc".to_string()),
            git_dir: None,
        }
    }

    fn loaded_index(symbols: Vec<IndexedSymbol>) -> index::LoadedIndex {
        let files = vec![
            IndexedFile {
                path: "src/types.rs".to_string(),
                role: FileRole::Source,
                size_bytes: Some(100),
                modified_unix: Some(1),
                content_hash: Some("aa".to_string()),
            },
            IndexedFile {
                path: "src/search.rs".to_string(),
                role: FileRole::Source,
                size_bytes: Some(100),
                modified_unix: Some(1),
                content_hash: Some("bb".to_string()),
            },
        ];
        let edges = vec![
            IndexedEdge {
                edge_type: "imports".to_string(),
                from: "src/search.rs".to_string(),
                to: "src/types.rs".to_string(),
                confidence: EdgeConfidence::Extracted,
                reason: "imports crate::types".to_string(),
            },
            IndexedEdge {
                edge_type: "same_area".to_string(),
                from: "src/types.rs".to_string(),
                to: "src/search.rs".to_string(),
                confidence: EdgeConfidence::Extracted,
                reason: "shared source area src".to_string(),
            },
        ];

        index::LoadedIndex {
            index_path: PathBuf::from("C:/repo/.agentgrep/index.json"),
            state: IndexState::Fresh,
            index: Some(RepoIndex {
                schema_version: crate::index::INDEX_SCHEMA_VERSION,
                repo_root: "C:/repo".to_string(),
                repo_rev: Some("abc".to_string()),
                indexed_at_unix: 1,
                files,
                symbols,
                edges,
                stats: IndexStats {
                    file_count: 2,
                    role_counts: std::collections::BTreeMap::from([(FileRole::Source, 2)]),
                    symbol_count: 0,
                    symbol_kind_counts: std::collections::BTreeMap::new(),
                    connection_count: 2,
                },
            }),
        }
    }

    fn symbol(name: &str, file_path: &str, line_number: usize) -> IndexedSymbol {
        IndexedSymbol {
            name: name.to_string(),
            kind: crate::types::SymbolKind::Function,
            file_path: file_path.to_string(),
            line_number,
            visibility: crate::types::Visibility::Public,
            signature: Some(format!("pub fn {name}()")),
        }
    }

    #[test]
    fn exact_symbol_lookup_prefers_exact_matches() {
        let loaded = loaded_index(vec![symbol("SearchResult", "src/types.rs", 12)]);
        let report = build_report_from_loaded(&repo(), &loaded, "SearchResult").unwrap();

        assert_eq!(report.match_mode, SymbolMatchMode::Exact);
        assert_eq!(report.matches.len(), 1);
        assert_eq!(report.matches[0].symbol.name, "SearchResult");
        assert_eq!(report.matches[0].file_role, "source");
        assert!(report.matches[0]
            .next_actions
            .iter()
            .any(|action| action == "agentgrep map src/types.rs"));
    }

    #[test]
    fn case_insensitive_lookup_works() {
        let loaded = loaded_index(vec![symbol("FindReport", "src/types.rs", 22)]);
        let report = build_report_from_loaded(&repo(), &loaded, "findreport").unwrap();

        assert_eq!(report.match_mode, SymbolMatchMode::CaseInsensitive);
        assert_eq!(report.matches.len(), 1);
        assert_eq!(report.matches[0].symbol.name, "FindReport");
    }

    #[test]
    fn partial_fallback_lookup_returns_substrings() {
        let loaded = loaded_index(vec![
            symbol("fixture_helper", "src/types.rs", 8),
            symbol("SearchResult", "src/types.rs", 12),
        ]);
        let report = build_report_from_loaded(&repo(), &loaded, "fixture").unwrap();

        assert_eq!(report.match_mode, SymbolMatchMode::Substring);
        assert!(report
            .matches
            .iter()
            .any(|item| item.symbol.name == "fixture_helper"));
    }

    #[test]
    fn missing_index_report_contains_actions() {
        let repo = repo();
        let loaded = index::LoadedIndex {
            index_path: PathBuf::from("C:/repo/.agentgrep/index.json"),
            state: IndexState::Missing,
            index: None,
        };
        let report = build_report_from_loaded(&repo, &loaded, "SearchResult").unwrap();

        assert_eq!(report.index_status, "missing");
        assert_eq!(report.match_mode, SymbolMatchMode::None);
        assert!(report
            .next_actions
            .iter()
            .any(|action| action == "agentgrep index"));
    }

    #[test]
    fn json_shape_includes_symbol_and_file_context() {
        let loaded = loaded_index(vec![symbol("SearchResult", "src/types.rs", 12)]);
        let report = build_report_from_loaded(&repo(), &loaded, "SearchResult").unwrap();

        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["query"], "SearchResult");
        assert_eq!(json["matches"].as_array().unwrap().len(), 1);
        assert_eq!(json["matches"][0]["symbol"]["name"], "SearchResult");
        assert_eq!(json["matches"][0]["file_role"], "source");
        assert!(json["matches"][0]["outgoing_edges"].is_array());
        assert!(json["matches"][0]["incoming_edges"].is_array());
    }

    #[test]
    fn keeps_fixture_symbols_without_special_filtering() {
        let loaded = loaded_index(vec![symbol("fixture_helper", "src/types.rs", 8)]);
        let report = build_report_from_loaded(&repo(), &loaded, "fixture").unwrap();

        assert!(report
            .matches
            .iter()
            .any(|item| item.symbol.name == "fixture_helper"));
    }
}
