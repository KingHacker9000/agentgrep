use tree_sitter::Node;

use crate::index::EdgeConfidence;
use crate::types::{SymbolKind, Visibility};

use super::extracted::{edge, file_facts, symbol, FileFacts, ImportBinding};
use super::language::{normalize_path, parent_dir, parse_source, RepoLookup};
use super::symbol_signature;

pub fn extract_file_facts(file_path: &str, source: &str, lookup: &RepoLookup) -> FileFacts {
    let Some(tree) = parse_source(tree_sitter_python::LANGUAGE.into(), source) else {
        return file_facts();
    };

    let root = tree.root_node();
    let mut facts = file_facts();
    let mut cursor = root.walk();
    for child in root.named_children(&mut cursor) {
        walk_python_node(child, file_path, source, lookup, &mut facts);
    }
    facts
}

fn walk_python_node(
    node: Node,
    file_path: &str,
    source: &str,
    lookup: &RepoLookup,
    facts: &mut FileFacts,
) {
    match node.kind() {
        "import_statement" => {
            for child in named_children(node) {
                if let Some(module_text) = python_module_text(child, source) {
                    if let Some(target) = resolve_python_module(file_path, &module_text, lookup) {
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
        "import_from_statement" => {
            let module_node = node.child_by_field_name("module_name");
            let module_text = module_node
                .and_then(|value| value.utf8_text(source.as_bytes()).ok())
                .map(|value| value.trim().to_string())
                .unwrap_or_default();

            if let Some(target) = resolve_python_module(file_path, &module_text, lookup) {
                facts.edges.push(edge(
                    "imports",
                    file_path,
                    &target,
                    EdgeConfidence::Extracted,
                    format!("imports {}", normalize_path(&target)),
                ));

                for imported_name in python_import_names(node, source) {
                    facts.symbol_references.push(ImportBinding {
                        from_file: file_path.to_string(),
                        symbol_name: imported_name,
                        target_file: Some(target.clone()),
                        line_number: node.start_position().row + 1,
                        confidence: EdgeConfidence::Extracted,
                        reason: format!("direct import binding from {}", normalize_path(&target)),
                    });
                }
            } else if module_text.chars().all(|ch| ch == '.') {
                for imported_name in python_import_names(node, source) {
                    if let Some(target) = resolve_relative_python_name(
                        file_path,
                        &module_text,
                        &imported_name,
                        lookup,
                    ) {
                        facts.edges.push(edge(
                            "imports",
                            file_path,
                            &target,
                            EdgeConfidence::Inferred,
                            format!("imports {}", normalize_path(&target)),
                        ));
                    }
                }
            }
        }
        "decorated_definition" => {
            for child in named_children(node) {
                walk_python_node(child, file_path, source, lookup, facts);
            }
        }
        "class_definition" => {
            if let Some(name) = node
                .child_by_field_name("name")
                .and_then(|node| node.utf8_text(source.as_bytes()).ok())
            {
                facts.symbols.push(symbol(
                    name.to_string(),
                    SymbolKind::Struct,
                    file_path,
                    node.start_position().row + 1,
                    python_visibility(name),
                    symbol_signature(source, node.start_position().row + 1, 120),
                ));
            }
        }
        "function_definition" => {
            if let Some(name) = node
                .child_by_field_name("name")
                .and_then(|node| node.utf8_text(source.as_bytes()).ok())
            {
                facts.symbols.push(symbol(
                    name.to_string(),
                    SymbolKind::Function,
                    file_path,
                    node.start_position().row + 1,
                    python_visibility(name),
                    symbol_signature(source, node.start_position().row + 1, 120),
                ));
            }
        }
        _ => {
            for child in named_children(node) {
                walk_python_node(child, file_path, source, lookup, facts);
            }
        }
    }
}

fn python_module_text(node: Node, source: &str) -> Option<String> {
    match node.kind() {
        "dotted_name" => node
            .utf8_text(source.as_bytes())
            .ok()
            .map(|value| value.trim().to_string()),
        "aliased_import" => node
            .child_by_field_name("name")
            .and_then(|child| child.utf8_text(source.as_bytes()).ok())
            .map(|value| value.trim().to_string())
            .or_else(|| {
                node.child(0)
                    .and_then(|child| child.utf8_text(source.as_bytes()).ok())
                    .map(|value| value.trim().to_string())
            }),
        _ => node
            .utf8_text(source.as_bytes())
            .ok()
            .map(|value| value.trim().to_string()),
    }
}

fn python_import_names(node: Node, source: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "aliased_import" || child.kind() == "dotted_name" {
            if let Some(text) = child.utf8_text(source.as_bytes()).ok() {
                let mut candidate = text.trim().to_string();
                if let Some((before_as, _)) = candidate.split_once(" as ") {
                    candidate = before_as.trim().to_string();
                }
                if !candidate.is_empty() {
                    names.push(candidate);
                }
            }
        }
    }
    names
}

fn resolve_python_module(
    file_path: &str,
    module_text: &str,
    lookup: &RepoLookup,
) -> Option<String> {
    let module_text = module_text.trim();
    if module_text.is_empty() {
        return None;
    }

    let (base_dir, remainder) = if module_text.starts_with('.') {
        let mut rest = module_text;
        let mut base_dir = parent_dir(file_path);
        let mut dots = 0usize;
        while rest.starts_with('.') {
            dots += 1;
            rest = &rest[1..];
        }
        for _ in 1..dots {
            base_dir = parent_dir(&base_dir);
        }
        (base_dir, rest.trim_matches('.'))
    } else {
        (String::new(), module_text)
    };

    let suffix = remainder.replace('.', "/");
    let mut candidates = Vec::new();
    if suffix.is_empty() {
        candidates.push(join_candidate(&base_dir, "__init__.py"));
    } else {
        if module_text.starts_with('.') {
            candidates.push(join_candidate(&base_dir, &format!("{suffix}.py")));
            candidates.push(join_candidate(&base_dir, &format!("{suffix}/__init__.py")));
        } else if suffix.contains('/') {
            candidates.push(join_candidate(&base_dir, &format!("{suffix}.py")));
            candidates.push(join_candidate(&base_dir, &format!("{suffix}/__init__.py")));
        } else {
            candidates.push(join_candidate(&base_dir, &format!("{suffix}/__init__.py")));
            candidates.push(join_candidate(&base_dir, &format!("{suffix}.py")));
        }
    }

    lookup.resolve_candidates(candidates)
}

fn resolve_relative_python_name(
    file_path: &str,
    module_text: &str,
    imported_name: &str,
    lookup: &RepoLookup,
) -> Option<String> {
    let imported_name = imported_name.trim();
    if imported_name.is_empty() || imported_name == "*" {
        return None;
    }

    let mut base_dir = parent_dir(file_path);
    let mut dots = 0usize;
    let mut rest = module_text;
    while rest.starts_with('.') {
        dots += 1;
        rest = &rest[1..];
    }
    for _ in 1..dots {
        base_dir = parent_dir(&base_dir);
    }

    if !rest.trim_matches('.').is_empty() {
        base_dir = join_base(&base_dir, &rest.replace('.', "/"));
    }
    resolve_python_name_at_base(&base_dir, imported_name, lookup)
}

fn resolve_python_name_at_base(base_dir: &str, name: &str, lookup: &RepoLookup) -> Option<String> {
    let mut candidates = Vec::new();
    candidates.push(join_candidate(base_dir, &format!("{name}.py")));
    candidates.push(join_candidate(base_dir, &format!("{name}/__init__.py")));
    lookup.resolve_candidates(candidates)
}

fn join_candidate(base_dir: &str, suffix: &str) -> String {
    if base_dir.is_empty() {
        suffix.trim_start_matches('/').to_string()
    } else {
        format!("{base_dir}/{suffix}")
    }
}

fn join_base(base_dir: &str, suffix: &str) -> String {
    if base_dir.is_empty() {
        suffix.to_string()
    } else if suffix.is_empty() {
        base_dir.to_string()
    } else {
        format!("{base_dir}/{suffix}")
    }
}

fn python_visibility(name: &str) -> Visibility {
    if name.starts_with('_') {
        Visibility::Private
    } else {
        Visibility::Public
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
    fn extracts_python_symbols_and_import_edges() {
        let source = r#"
import app.models
from app.llm_client import LLMClient
from .schemas import Foo
from .meeting_session import start_session as launch_session

class SessionLiveStateLoop:
    pass

async def start_session():
    return Foo()
"#;
        let facts = extract_file_facts(
            "app/meeting_session.py",
            source,
            &lookup(&[
                "app/meeting_session.py",
                "app/models.py",
                "app/schemas.py",
                "app/llm_client.py",
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
            .any(|edge| edge.to == "app/models.py" && edge.edge_type == "imports"));
        assert!(facts
            .edges
            .iter()
            .any(|edge| edge.to == "app/schemas.py" && edge.edge_type == "imports"));
        assert!(facts.symbol_references.iter().any(|binding| {
            binding.symbol_name == "LLMClient"
                && binding.target_file.as_deref() == Some("app/llm_client.py")
        }));
        assert!(facts.symbol_references.iter().any(|binding| {
            binding.symbol_name == "start_session"
                && binding.target_file.as_deref() == Some("app/meeting_session.py")
        }));
    }
}
