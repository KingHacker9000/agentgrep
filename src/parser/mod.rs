pub mod extracted;
pub mod go;
pub mod javascript;
pub mod language;
pub mod python;
pub mod rust;
pub mod typescript;

use std::collections::BTreeMap;

use crate::index::{self, FileRole, IndexedFile};

use extracted::{dedup_edges, dedup_symbols, RepoFacts};
use language::{detect_language, RepoLookup};

pub fn extract_repo_facts(
    files: &[IndexedFile],
    source_texts: &BTreeMap<String, String>,
) -> RepoFacts {
    let lookup = RepoLookup::new(files);
    let mut facts = RepoFacts::default();

    for file in files
        .iter()
        .filter(|file| matches!(file.role, FileRole::Source))
    {
        let Some(language) = detect_language(&file.path) else {
            continue;
        };
        let Some(source) = source_texts.get(&file.path) else {
            continue;
        };

        let file_facts = match language {
            language::LanguageKind::Rust => rust::extract_file_facts(&file.path, source, &lookup),
            language::LanguageKind::Go => go::extract_file_facts(&file.path, source, &lookup),
            language::LanguageKind::Python => {
                python::extract_file_facts(&file.path, source, &lookup)
            }
            language::LanguageKind::JavaScript => {
                javascript::extract_file_facts(&file.path, source, &lookup)
            }
            language::LanguageKind::TypeScript => {
                typescript::extract_file_facts(&file.path, source, &lookup)
            }
            language::LanguageKind::Tsx => {
                typescript::extract_file_facts(&file.path, source, &lookup)
            }
        };

        facts.merge_file(language, file_facts);
    }

    dedup_symbols(&mut facts.symbols);
    dedup_edges(&mut facts.edges);
    facts
}

pub fn combine_with_rust_fallback(
    mut facts: RepoFacts,
    files: &[IndexedFile],
    source_texts: &BTreeMap<String, String>,
) -> RepoFacts {
    if facts.rust_file_count > 0 && facts.rust_symbol_count == 0 && facts.rust_edge_count == 0 {
        let rust_symbols = index::build_rust_symbols(files, source_texts);
        let rust_edges = index::build_rust_edges(files, source_texts);
        facts.symbols.extend(rust_symbols);
        facts.edges.extend(rust_edges);
        dedup_symbols(&mut facts.symbols);
        dedup_edges(&mut facts.edges);
    }

    facts
}

pub(crate) fn source_line(source: &str, line_number: usize) -> Option<&str> {
    if line_number == 0 {
        return None;
    }
    source.lines().nth(line_number - 1)
}

pub(crate) fn symbol_signature(source: &str, line_number: usize, max_len: usize) -> Option<String> {
    let line = source_line(source, line_number)?.trim();
    if line.is_empty() {
        None
    } else {
        Some(crate::text::shorten_snippet(line, max_len))
    }
}
