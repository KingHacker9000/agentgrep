/// Query-time resolution: given a symbol that has no local definition,
/// try to determine which external dependency provides it by scanning
/// call site files for type annotations on the receiver.
///
/// Strategy (text-based, no tree-sitter at query time):
/// 1. For each call site, find the line and extract the receiver identifier
///    (the token immediately before `.symbol_name(...)`).
/// 2. Scan the full file for declarations that bind that receiver to a type:
///       Rust:   `let receiver: Type`  /  `receiver: Type,`  /  `receiver: Type )`
///       Python: `receiver: Type`  /  `receiver = Type(`
/// 3. Look the type up in `dep_imports`.
/// 4. Return the first match found across up to MAX_FILES files.
use std::path::Path;

use crate::index::RepoIndex;
use crate::types::DepImport;

const MAX_FILES: usize = 4;

/// Given a symbol not found in the index, scan up to MAX_FILES call site files
/// and attempt to identify which external package provides it via type annotations.
/// Returns the dep_package string if found.
pub fn resolve_dep_for_symbol(
    symbol_name: &str,
    index: &RepoIndex,
    repo_root: &Path,
) -> Option<String> {
    // Collect distinct call site files (production callers first).
    let call_files: Vec<String> = {
        let mut seen = std::collections::HashSet::new();
        let mut files = Vec::new();
        for r in &index.symbol_references {
            if r.symbol_name.eq_ignore_ascii_case(symbol_name) && seen.insert(r.from_file.clone())
            {
                files.push(r.from_file.clone());
                if files.len() >= MAX_FILES {
                    break;
                }
            }
        }
        files
    };

    for file_path in &call_files {
        let abs = repo_root.join(file_path.replace('\\', "/"));
        let Ok(source) = std::fs::read_to_string(&abs) else {
            continue;
        };
        if let Some(pkg) = scan_file_for_dep(&source, symbol_name, &index.dep_imports) {
            return Some(pkg);
        }
    }

    None
}

/// Scan a single source file for type annotations that link the receiver of
/// `symbol_name` to a known dep_import entry.
fn scan_file_for_dep(source: &str, symbol_name: &str, dep_imports: &[DepImport]) -> Option<String> {
    // Find all lines that contain `.symbol_name(` or `.symbol_name` at word boundary.
    let call_pattern = format!(".{symbol_name}");
    let lines: Vec<&str> = source.lines().collect();

    let mut candidate_receivers: Vec<String> = Vec::new();

    for line in &lines {
        if !line.contains(&call_pattern) {
            continue;
        }
        // Extract the receiver: the word immediately before `.symbol_name`
        if let Some(recv) = extract_receiver(line, symbol_name) {
            if !recv.is_empty() && !candidate_receivers.contains(&recv) {
                candidate_receivers.push(recv);
            }
        }
    }

    // For each receiver, scan the whole file for its type declaration.
    for receiver in &candidate_receivers {
        if let Some(type_name) = find_receiver_type(receiver, &lines) {
            // Look up the type in dep_imports
            if let Some(pkg) = lookup_dep_package(&type_name, dep_imports) {
                return Some(pkg);
            }
        }
    }

    None
}

/// Extract the receiver identifier from a call line.
/// e.g. `syntax_set.find_syntax_for_file(path)` → `syntax_set`
///      `self.syntax_set.find(...)` → `syntax_set` (innermost before the method)
fn extract_receiver(line: &str, symbol_name: &str) -> Option<String> {
    let call = format!(".{symbol_name}");
    let pos = line.find(&call)?;
    // Walk backwards from pos to find the identifier
    let before = &line[..pos];
    let recv = before
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|s| !s.is_empty())
        .last()?;
    Some(recv.to_string())
}

/// Scan lines for `receiver: TypeName` or `receiver = TypeName(` patterns.
fn find_receiver_type(receiver: &str, lines: &[&str]) -> Option<String> {
    // Rust/Python type annotation: `receiver: TypeName`
    let annotation_pattern = format!("{receiver}:");
    // Python / Rust constructor: `receiver = TypeName(`
    let constructor_pattern = format!("{receiver} = ");

    for line in lines {
        let trimmed = line.trim();

        if let Some(after) = trimmed
            .find(&annotation_pattern)
            .map(|pos| &trimmed[pos + annotation_pattern.len()..])
        {
            let type_name = extract_type_name(after.trim());
            if !type_name.is_empty() {
                return Some(type_name);
            }
        }

        if let Some(after) = trimmed
            .find(&constructor_pattern)
            .map(|pos| &trimmed[pos + constructor_pattern.len()..])
        {
            // `TypeName(` or `TypeName {` — extract TypeName
            let type_name = after
                .trim()
                .split(|c: char| !c.is_alphanumeric() && c != '_')
                .next()
                .unwrap_or("")
                .to_string();
            if !type_name.is_empty() {
                return Some(type_name);
            }
        }
    }

    None
}

/// Extract a clean type name from the text after `: `, stripping generics,
/// lifetimes, references, and Option/Box wrappers.
fn extract_type_name(text: &str) -> String {
    // Strip leading `&`, `mut `, lifetime `'a `
    let text = text.trim_start_matches('&').trim();
    let text = text.strip_prefix("mut ").unwrap_or(text).trim();
    // Strip lifetime: `'a TypeName`
    let text = if text.starts_with('\'') {
        text.splitn(2, ' ').nth(1).unwrap_or("").trim()
    } else {
        text
    };
    // Strip Option<T> / Box<T> / Arc<T> / Rc<T> wrappers — dive into first generic arg
    for wrapper in &["Option<", "Box<", "Arc<", "Rc<", "Vec<"] {
        if let Some(inner) = text.strip_prefix(wrapper) {
            let inner = inner.trim_end_matches('>').trim();
            return extract_type_name(inner);
        }
    }
    // Take the base identifier before `<`, `(`, `,`, `)`, `>`, ` `
    text.split(|c: char| !c.is_alphanumeric() && c != '_')
        .next()
        .unwrap_or("")
        .to_string()
}

/// Look up a type name in dep_imports, returning the dep_package if found.
fn lookup_dep_package(type_name: &str, dep_imports: &[DepImport]) -> Option<String> {
    dep_imports
        .iter()
        .find(|d| d.symbol_or_module.eq_ignore_ascii_case(type_name))
        .map(|d| d.dep_package.clone())
}
