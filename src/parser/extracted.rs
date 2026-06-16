use std::collections::HashSet;

use crate::index::{EdgeConfidence, IndexedEdge};
use crate::types::{IndexedSymbol, SymbolKind, Visibility};

use super::language::LanguageKind;

#[derive(Debug, Default, Clone)]
pub struct FileFacts {
    pub symbols: Vec<IndexedSymbol>,
    pub edges: Vec<IndexedEdge>,
}

#[derive(Debug, Default, Clone)]
pub struct RepoFacts {
    pub symbols: Vec<IndexedSymbol>,
    pub edges: Vec<IndexedEdge>,
    pub rust_file_count: usize,
    pub rust_symbol_count: usize,
    pub rust_edge_count: usize,
}

impl RepoFacts {
    pub fn merge_file(&mut self, language: LanguageKind, facts: FileFacts) {
        if matches!(language, LanguageKind::Rust) {
            self.rust_file_count += 1;
            self.rust_symbol_count += facts.symbols.len();
            self.rust_edge_count += facts.edges.len();
        }

        self.symbols.extend(facts.symbols);
        self.edges.extend(facts.edges);
    }
}

pub fn file_facts() -> FileFacts {
    FileFacts::default()
}

pub fn symbol(
    name: String,
    kind: SymbolKind,
    file_path: &str,
    line_number: usize,
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

pub fn edge(
    edge_type: &str,
    from: &str,
    to: &str,
    confidence: EdgeConfidence,
    reason: String,
) -> IndexedEdge {
    IndexedEdge {
        edge_type: edge_type.to_string(),
        from: from.to_string(),
        to: to.to_string(),
        confidence,
        reason,
    }
}

pub fn dedup_symbols(symbols: &mut Vec<IndexedSymbol>) {
    let mut seen = HashSet::new();
    symbols.retain(|symbol| {
        seen.insert((
            symbol.name.clone(),
            symbol.kind.clone(),
            symbol.file_path.clone(),
            symbol.line_number,
            symbol.visibility.clone(),
            symbol.signature.clone(),
        ))
    });
}

pub fn dedup_edges(edges: &mut Vec<IndexedEdge>) {
    let mut seen = HashSet::new();
    edges.retain(|edge| {
        seen.insert((
            edge.edge_type.clone(),
            edge.from.clone(),
            edge.to.clone(),
            edge.confidence.clone(),
            edge.reason.clone(),
        ))
    });
}
