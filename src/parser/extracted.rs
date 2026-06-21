use std::collections::HashSet;

use crate::index::{EdgeConfidence, IndexedEdge, IndexedSymbolReference, ReferenceContext};
use crate::types::{IndexedSymbol, SymbolKind, Visibility};

use super::language::LanguageKind;

#[derive(Debug, Default, Clone)]
pub struct FileFacts {
    pub symbols: Vec<IndexedSymbol>,
    pub edges: Vec<IndexedEdge>,
    pub symbol_references: Vec<ImportBinding>,
}

#[derive(Debug, Default, Clone)]
pub struct RepoFacts {
    pub symbols: Vec<IndexedSymbol>,
    pub edges: Vec<IndexedEdge>,
    pub symbol_references: Vec<ImportBinding>,
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
        self.symbol_references.extend(facts.symbol_references);
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
        end_line: None,
        parent_class: None,
    }
}

pub fn symbol_with_extent(
    name: String,
    kind: SymbolKind,
    file_path: &str,
    line_number: usize,
    end_line: usize,
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
        end_line: Some(end_line),
        parent_class: None,
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

#[derive(Debug, Clone)]
pub struct ImportBinding {
    pub from_file: String,
    pub symbol_name: String,
    pub target_file: Option<String>,
    pub line_number: usize,
    pub confidence: EdgeConfidence,
    pub reason: String,
}

impl ImportBinding {
    pub fn into_reference(self, target_line: Option<usize>) -> IndexedSymbolReference {
        IndexedSymbolReference {
            from_file: self.from_file,
            symbol_name: self.symbol_name,
            target_file: self.target_file,
            target_line,
            line_number: self.line_number,
            confidence: self.confidence,
            reason: self.reason,
            context: ReferenceContext::Production,
            additional_count: 0,
        }
    }
}

pub fn call_site(symbol_name: &str, from_file: &str, line_number: usize) -> ImportBinding {
    ImportBinding {
        from_file: from_file.to_string(),
        symbol_name: symbol_name.to_string(),
        target_file: None,
        line_number,
        confidence: EdgeConfidence::Inferred,
        reason: "call site".to_string(),
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
