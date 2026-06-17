use tree_sitter::Node;

use crate::index::EdgeConfidence;
use crate::types::{SymbolKind, Visibility};

use super::extracted::{edge, file_facts, symbol, FileFacts};
use super::language::{normalize_path, parent_dir, parse_source, RepoLookup};
use super::symbol_signature;

pub fn extract_file_facts(file_path: &str, source: &str, lookup: &RepoLookup) -> FileFacts {
    let Some(tree) = parse_source(tree_sitter_go::LANGUAGE.into(), source) else {
        return file_facts();
    };

    let root = tree.root_node();
    let mut facts = file_facts();
    let mut cursor = root.walk();
    for child in root.named_children(&mut cursor) {
        walk_go_node(child, file_path, source, lookup, &mut facts);
    }
    facts
}

fn walk_go_node(
    node: Node,
    file_path: &str,
    source: &str,
    lookup: &RepoLookup,
    facts: &mut FileFacts,
) {
    match node.kind() {
        "import_declaration" => {
            for child in named_children(node) {
                if child.kind() == "import_spec" {
                    if let Some(import_path) = go_import_path(child, source) {
                        if let Some(target) = resolve_go_import(file_path, &import_path, lookup) {
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
        }
        "function_declaration" => {
            if let Some(name) = node
                .child_by_field_name("name")
                .and_then(|node| node.utf8_text(source.as_bytes()).ok())
            {
                facts.symbols.push(symbol(
                    name.to_string(),
                    SymbolKind::Function,
                    file_path,
                    node.start_position().row + 1,
                    Visibility::Public,
                    symbol_signature(source, node.start_position().row + 1, 120),
                ));
            }
        }
        "method_declaration" => {
            if let Some(name) = node
                .child_by_field_name("name")
                .and_then(|node| node.utf8_text(source.as_bytes()).ok())
            {
                facts.symbols.push(symbol(
                    name.to_string(),
                    SymbolKind::Function,
                    file_path,
                    node.start_position().row + 1,
                    Visibility::Public,
                    symbol_signature(source, node.start_position().row + 1, 120),
                ));
            }
        }
        "type_spec" => {
            let Some(name) = node
                .child_by_field_name("name")
                .and_then(|node| node.utf8_text(source.as_bytes()).ok())
            else {
                for child in named_children(node) {
                    walk_go_node(child, file_path, source, lookup, facts);
                }
                return;
            };

            let kind = match node
                .child_by_field_name("type")
                .map(|node| node.kind())
                .unwrap_or_default()
            {
                "struct_type" => SymbolKind::Struct,
                "interface_type" => SymbolKind::Trait,
                _ => SymbolKind::TypeAlias,
            };

            facts.symbols.push(symbol(
                name.to_string(),
                kind,
                file_path,
                node.start_position().row + 1,
                Visibility::Public,
                symbol_signature(source, node.start_position().row + 1, 120),
            ));
        }
        "const_spec" => {
            for name in go_spec_names(node, source) {
                facts.symbols.push(symbol(
                    name,
                    SymbolKind::Const,
                    file_path,
                    node.start_position().row + 1,
                    Visibility::Public,
                    symbol_signature(source, node.start_position().row + 1, 120),
                ));
            }
        }
        "var_spec" => {
            for name in go_spec_names(node, source) {
                facts.symbols.push(symbol(
                    name,
                    SymbolKind::Static,
                    file_path,
                    node.start_position().row + 1,
                    Visibility::Public,
                    symbol_signature(source, node.start_position().row + 1, 120),
                ));
            }
        }
        _ => {
            for child in named_children(node) {
                walk_go_node(child, file_path, source, lookup, facts);
            }
        }
    }
}

fn resolve_go_import(file_path: &str, import_path: &str, lookup: &RepoLookup) -> Option<String> {
    let import_path = import_path
        .trim()
        .trim_start_matches("./")
        .trim_start_matches("../");
    if import_path.is_empty() {
        return None;
    }

    let mut candidates = Vec::new();
    let normalized = import_path.replace('\\', "/");
    let base_dir = parent_dir(file_path);
    if normalized.contains('/') {
        push_go_candidates(&mut candidates, &normalized);
    }
    if !base_dir.is_empty() {
        let joined = format!("{base_dir}/{normalized}");
        push_go_candidates(&mut candidates, &joined);
    }
    push_go_candidates(&mut candidates, &normalized);

    lookup.resolve_candidates(candidates)
}

fn push_go_candidates(candidates: &mut Vec<String>, path: &str) {
    let stripped = path.trim_matches('/');
    if stripped.is_empty() {
        return;
    }

    let segments = stripped.split('/').collect::<Vec<_>>();
    for start in 0..segments.len() {
        let suffix = segments[start..].join("/");
        candidates.push(format!("{suffix}.go"));
        candidates.push(format!("{suffix}/index.go"));
        if let Some(last) = segments.last() {
            candidates.push(format!("{suffix}/{last}.go"));
        }
    }

    candidates.push(stripped.to_string());
}

fn go_import_path(node: Node, source: &str) -> Option<String> {
    let text = node.utf8_text(source.as_bytes()).ok()?.trim();
    let bytes = text.as_bytes();
    let first_quote = bytes
        .iter()
        .position(|byte| *byte == b'"' || *byte == b'`')?;
    let quote = bytes[first_quote];
    let rest = &text[first_quote + 1..];
    let end = rest.find(quote as char)?;
    Some(rest[..end].to_string())
}

fn go_spec_names(node: Node, source: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "identifier" {
            if let Some(name) = child.utf8_text(source.as_bytes()).ok() {
                names.push(name.to_string());
            }
        }
    }
    names
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
    fn extracts_go_symbols_and_import_edges() {
        let source = r#"
package cli

import "github.com/acme/quickget/pkg/quickget/runtime"

type Runner struct {}
type Processor interface {}

const Version = "1.0.0"
var DefaultRunner = Runner{}

func Execute() {}

func (r *Runner) Start() {}
"#;
        let facts = extract_file_facts(
            "cmd/quickget-agent/main.go",
            source,
            &lookup(&[
                "cmd/quickget-agent/main.go",
                "pkg/quickget/runtime/runtime.go",
            ]),
        );

        assert!(facts.symbols.iter().any(|symbol| symbol.name == "Runner"));
        assert!(facts
            .symbols
            .iter()
            .any(|symbol| symbol.name == "Processor"));
        assert!(facts.symbols.iter().any(|symbol| symbol.name == "Version"));
        assert!(facts
            .symbols
            .iter()
            .any(|symbol| symbol.name == "DefaultRunner"));
        assert!(facts.symbols.iter().any(|symbol| symbol.name == "Execute"));
        assert!(facts
            .edges
            .iter()
            .any(|edge| edge.to == "pkg/quickget/runtime/runtime.go"));
    }
}
