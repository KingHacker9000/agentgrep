use std::path::Path;

use tree_sitter::Node;

use crate::index::EdgeConfidence;
use crate::types::{SymbolKind, Visibility};

use super::extracted::{edge, file_facts, symbol, FileFacts};
use super::language::{join_relative, normalize_path, parent_dir, parse_source, RepoLookup};
use super::symbol_signature;

pub fn extract_file_facts(file_path: &str, source: &str, lookup: &RepoLookup) -> FileFacts {
    let language = if file_path.to_lowercase().ends_with(".tsx") {
        tree_sitter_typescript::LANGUAGE_TSX
    } else {
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT
    };
    let Some(tree) = parse_source(language.into(), source) else {
        return file_facts();
    };

    let root = tree.root_node();
    let mut facts = file_facts();
    let mut cursor = root.walk();
    for child in root.named_children(&mut cursor) {
        walk_ts_node(child, file_path, source, lookup, false, &mut facts);
    }
    facts
}

fn walk_ts_node(
    node: Node,
    file_path: &str,
    source: &str,
    lookup: &RepoLookup,
    exported: bool,
    facts: &mut FileFacts,
) {
    match node.kind() {
        "import_statement" => {
            if let Some(source_node) = node.child_by_field_name("source") {
                if let Some(target) = resolve_ts_import(file_path, source, lookup, source_node) {
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
        "export_statement" => {
            if let Some(source_node) = node.child_by_field_name("source") {
                if let Some(target) = resolve_ts_import(file_path, source, lookup, source_node) {
                    facts.edges.push(edge(
                        "imports",
                        file_path,
                        &target,
                        EdgeConfidence::Extracted,
                        format!("imports {}", normalize_path(&target)),
                    ));
                }
            }

            for child in named_children(node) {
                walk_ts_node(child, file_path, source, lookup, true, facts);
            }
        }
        "call_expression" => {
            if is_require_call(node, source) {
                if let Some(target_node) = first_string_argument(node) {
                    if let Some(target) = resolve_ts_import(file_path, source, lookup, target_node)
                    {
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
        }
        "function_declaration" | "generator_function_declaration" => {
            if let Some(name) = node
                .child_by_field_name("name")
                .and_then(|node| node.utf8_text(source.as_bytes()).ok())
            {
                facts.symbols.push(symbol(
                    name.to_string(),
                    SymbolKind::Function,
                    file_path,
                    node.start_position().row + 1,
                    ts_visibility(name, exported),
                    symbol_signature(source, node.start_position().row + 1, 120),
                ));
            }
        }
        "class_declaration" => {
            if let Some(name) = node
                .child_by_field_name("name")
                .and_then(|node| node.utf8_text(source.as_bytes()).ok())
            {
                facts.symbols.push(symbol(
                    name.to_string(),
                    SymbolKind::Struct,
                    file_path,
                    node.start_position().row + 1,
                    ts_visibility(name, exported),
                    symbol_signature(source, node.start_position().row + 1, 120),
                ));
            }
        }
        "lexical_declaration" | "variable_declaration" => {
            for child in named_children(node) {
                if child.kind() == "variable_declarator" {
                    extract_variable_symbol(child, file_path, source, exported, facts);
                }
            }
        }
        _ => {
            for child in named_children(node) {
                walk_ts_node(child, file_path, source, lookup, exported, facts);
            }
        }
    }
}

fn extract_variable_symbol(
    node: Node,
    file_path: &str,
    source: &str,
    exported: bool,
    facts: &mut FileFacts,
) {
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let Some(name) = name_node.utf8_text(source.as_bytes()).ok() else {
        return;
    };
    let Some(initializer) = node.child_by_field_name("value") else {
        return;
    };

    let kind = match initializer.kind() {
        "arrow_function" | "function_expression" => SymbolKind::Function,
        "class_expression" => SymbolKind::Struct,
        _ => return,
    };

    facts.symbols.push(symbol(
        name.to_string(),
        kind,
        file_path,
        node.start_position().row + 1,
        ts_visibility(name, exported),
        symbol_signature(source, node.start_position().row + 1, 120),
    ));
}

fn resolve_ts_import(
    file_path: &str,
    source: &str,
    lookup: &RepoLookup,
    node: Node,
) -> Option<String> {
    let text = string_literal_text(node, source)?;
    if !text.starts_with('.') {
        return None;
    }

    let base_dir = parent_dir(file_path);
    let joined = join_relative(&base_dir, &text);
    let mut candidates = Vec::new();
    if Path::new(&joined).extension().is_some() {
        candidates.push(joined.clone());
    } else {
        for ext in [".ts", ".tsx", ".js", ".jsx"] {
            candidates.push(format!("{joined}{ext}"));
        }
        for ext in [".ts", ".tsx", ".js", ".jsx"] {
            candidates.push(format!("{joined}/index{ext}"));
        }
    }

    lookup.resolve_candidates(candidates)
}

fn string_literal_text(node: Node, source: &str) -> Option<String> {
    match node.kind() {
        "string" | "template_string" => {
            let raw = node.utf8_text(source.as_bytes()).ok()?.trim();
            Some(strip_quotes(raw))
        }
        "arguments" => {
            first_string_argument(node).and_then(|value| string_literal_text(value, source))
        }
        _ => node
            .utf8_text(source.as_bytes())
            .ok()
            .map(|value| strip_quotes(value.trim())),
    }
}

fn first_string_argument<'a>(node: Node<'a>) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "string" || child.kind() == "template_string" {
            return Some(child);
        }
        if child.kind() == "arguments" {
            if let Some(inner) = first_string_argument(child) {
                return Some(inner);
            }
        }
    }
    None
}

fn is_require_call(node: Node, source: &str) -> bool {
    let Some(function) = node.child_by_field_name("function") else {
        return false;
    };
    function.kind() == "identifier" && function.utf8_text(source.as_bytes()).ok() == Some("require")
}

fn strip_quotes(text: &str) -> String {
    let text = text.trim();
    if text.len() >= 2 {
        let bytes = text.as_bytes();
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'"' || first == b'\'' || first == b'`') && first == last {
            return text[1..text.len() - 1].to_string();
        }
    }
    text.to_string()
}

fn ts_visibility(name: &str, exported: bool) -> Visibility {
    if exported && !name.starts_with('_') {
        Visibility::Public
    } else {
        Visibility::Private
    }
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
            })
            .collect::<Vec<_>>();
        RepoLookup::new(&files)
    }

    #[test]
    fn extracts_typescript_symbols_and_import_edges() {
        let source = r#"
import helper from "./helper";
export { shared } from "../shared";
export function build() {}
const render = () => {};
"#;
        let facts = extract_file_facts(
            "src/app.ts",
            source,
            &lookup(&["src/app.ts", "src/helper.ts", "shared.ts"]),
        );
        assert!(facts.symbols.iter().any(|symbol| symbol.name == "build"));
        assert!(facts.symbols.iter().any(|symbol| symbol.name == "render"));
        assert!(facts.edges.iter().any(|edge| edge.to == "src/helper.ts"));
        assert!(facts.edges.iter().any(|edge| edge.to == "shared.ts"));
    }

    #[test]
    fn extracts_tsx_symbols_with_tsx_grammar() {
        let source = r#"
import React from "./react";
export class View {}
"#;
        let facts = extract_file_facts(
            "src/view.tsx",
            source,
            &lookup(&["src/view.tsx", "src/react.tsx"]),
        );
        assert!(facts.symbols.iter().any(|symbol| symbol.name == "View"));
    }
}
