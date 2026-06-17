use tree_sitter::Node;

use crate::index::EdgeConfidence;
use crate::types::{SymbolKind, Visibility};

use super::extracted::{edge, file_facts, symbol, FileFacts};
use super::language::{normalize_path, parse_source, RepoLookup};
use super::symbol_signature;

pub fn extract_file_facts(file_path: &str, source: &str, lookup: &RepoLookup) -> FileFacts {
    let Some(tree) = parse_source(tree_sitter_rust::LANGUAGE.into(), source) else {
        return file_facts();
    };

    let module_path = rust_module_path_components(file_path);
    let root = tree.root_node();
    let mut facts = file_facts();
    let mut cursor = root.walk();
    for child in root.named_children(&mut cursor) {
        walk_rust_node(child, file_path, source, lookup, &module_path, &mut facts);
    }
    facts
}

fn walk_rust_node(
    node: Node,
    file_path: &str,
    source: &str,
    lookup: &RepoLookup,
    module_path: &[String],
    facts: &mut FileFacts,
) {
    match node.kind() {
        "use_declaration" => {
            let line = node
                .utf8_text(source.as_bytes())
                .ok()
                .map(str::trim)
                .unwrap_or_default();
            if let Some(import_body) = crate::index::parse_rust_use_statement(line) {
                for path in crate::index::expand_rust_use_paths(import_body) {
                    if let Some((target, confidence)) =
                        resolve_rust_use_path(&path, module_path, lookup)
                    {
                        facts.edges.push(edge(
                            "imports",
                            file_path,
                            &target,
                            confidence,
                            format!("imports {}", normalize_path(&target)),
                        ));
                    }
                }
            }
        }
        "mod_item" => {
            let line = node
                .utf8_text(source.as_bytes())
                .ok()
                .map(str::trim)
                .unwrap_or_default();
            if let Some(module_name) = crate::index::parse_rust_mod_declaration(line) {
                let mut candidate = module_path.to_vec();
                candidate.push(module_name.clone());
                if let Some(target) = resolve_rust_module_path(&candidate, lookup) {
                    facts.edges.push(edge(
                        "imports",
                        file_path,
                        &target,
                        EdgeConfidence::Extracted,
                        format!("imports {}", normalize_path(&target)),
                    ));
                }
            }
        }
        "function_item" => push_rust_symbol(
            node,
            file_path,
            source,
            facts,
            SymbolKind::Function,
            crate::index::parse_rust_function_symbol,
        ),
        "struct_item" => push_rust_symbol(
            node,
            file_path,
            source,
            facts,
            SymbolKind::Struct,
            crate::index::parse_rust_struct_symbol,
        ),
        "enum_item" => push_rust_symbol(
            node,
            file_path,
            source,
            facts,
            SymbolKind::Enum,
            crate::index::parse_rust_enum_symbol,
        ),
        "trait_item" => push_rust_symbol(
            node,
            file_path,
            source,
            facts,
            SymbolKind::Trait,
            crate::index::parse_rust_trait_symbol,
        ),
        "impl_item" => push_rust_symbol(
            node,
            file_path,
            source,
            facts,
            SymbolKind::Impl,
            crate::index::parse_rust_impl_symbol,
        ),
        "const_item" => push_rust_symbol(
            node,
            file_path,
            source,
            facts,
            SymbolKind::Const,
            crate::index::parse_rust_const_symbol,
        ),
        "static_item" => push_rust_symbol(
            node,
            file_path,
            source,
            facts,
            SymbolKind::Static,
            crate::index::parse_rust_static_symbol,
        ),
        "attribute_item" | "inner_attribute_item" => {
            for child in named_children(node) {
                walk_rust_node(child, file_path, source, lookup, module_path, facts);
            }
        }
        _ => {
            for child in named_children(node) {
                walk_rust_node(child, file_path, source, lookup, module_path, facts);
            }
        }
    }
}

fn push_rust_symbol<F>(
    node: Node,
    file_path: &str,
    source: &str,
    facts: &mut FileFacts,
    kind: SymbolKind,
    parser: F,
) where
    F: Fn(&str) -> Option<String>,
{
    let line_number = node.start_position().row + 1;
    let line = node
        .utf8_text(source.as_bytes())
        .ok()
        .map(str::trim)
        .unwrap_or_default();
    let normalized_line = rust_item_line(line);
    let Some(name) = parser(normalized_line) else {
        return;
    };

    facts.symbols.push(symbol(
        name,
        kind,
        file_path,
        line_number,
        visibility_from_line(line),
        symbol_signature(source, line_number, 120),
    ));
}

fn visibility_from_line(line: &str) -> Visibility {
    if line.contains("pub ") || line.starts_with("pub(") {
        Visibility::Public
    } else {
        Visibility::Private
    }
}

fn rust_item_line(line: &str) -> &str {
    let (_, mut current) = crate::index::rust_visibility_prefix(line);
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

fn resolve_rust_module_path(candidate: &[String], lookup: &RepoLookup) -> Option<String> {
    for end in (1..=candidate.len()).rev() {
        let key = candidate[..end].join("/");
        let candidates = [
            format!("{key}.rs"),
            format!("{key}/mod.rs"),
            format!("src/{key}.rs"),
            format!("src/{key}/mod.rs"),
        ];
        if let Some(path) = lookup.resolve_candidates(candidates) {
            return Some(path);
        }
    }
    None
}

fn resolve_rust_use_path(
    candidate: &[String],
    current_module_path: &[String],
    lookup: &RepoLookup,
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
    resolve_rust_module_path(&combined, lookup).map(|target| {
        let confidence = if matches!(candidate.first().map(|part| part.as_str()), Some("crate")) {
            EdgeConfidence::Extracted
        } else {
            EdgeConfidence::Inferred
        };
        (target, confidence)
    })
}

fn rust_module_path_components(path: &str) -> Vec<String> {
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

fn named_children(node: Node) -> Vec<Node> {
    let mut children = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        children.push(child);
    }
    children
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::{FileRole, IndexedFile};

    fn lookup(paths: &[&str]) -> RepoLookup {
        let files = paths
            .iter()
            .map(|path| IndexedFile {
                path: (*path).to_string(),
                role: FileRole::Source,
                size_bytes: None,
                modified_unix: None,
                content_hash: None,
                ..Default::default()
            })
            .collect::<Vec<_>>();
        RepoLookup::new(&files)
    }

    #[test]
    fn extracts_rust_symbols_and_import_edges() {
        let source = r#"
pub mod models;
use crate::schemas::Session;

pub struct SessionLiveStateLoop;
pub fn start_session() {}
"#;
        let facts = extract_file_facts(
            "src/meeting_session.rs",
            source,
            &lookup(&[
                "src/meeting_session.rs",
                "src/meeting_session/models.rs",
                "src/schemas.rs",
            ]),
        );
        assert!(facts
            .symbols
            .iter()
            .any(|symbol| symbol.name == "SessionLiveStateLoop"));
        assert!(facts
            .symbols
            .iter()
            .any(|symbol| symbol.name == "start_session"));
        assert!(facts
            .edges
            .iter()
            .any(|edge| edge.to == "src/meeting_session/models.rs"));
        assert!(facts.edges.iter().any(|edge| edge.to == "src/schemas.rs"));
    }
}
