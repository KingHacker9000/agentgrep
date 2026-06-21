use tree_sitter::Node;

use crate::index::EdgeConfidence;
use crate::types::{SymbolKind, Visibility};

use super::extracted::{call_site, edge, file_facts, symbol, FileFacts, ImportBinding};
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
                .and_then(|n| n.utf8_text(source.as_bytes()).ok())
            {
                let mut sym = symbol(
                    name.to_string(),
                    SymbolKind::Struct,
                    file_path,
                    node.start_position().row + 1,
                    python_visibility(name),
                    symbol_signature(source, node.start_position().row + 1, 120),
                );
                sym.end_line = Some(node.end_position().row + 1);
                facts.symbols.push(sym);
                if let Some(body) = node.child_by_field_name("body") {
                    walk_python_class_body(body, name, file_path, source, lookup, facts);
                }
            }
        }
        "function_definition" => {
            if let Some(name) = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source.as_bytes()).ok())
            {
                let mut sym = symbol(
                    name.to_string(),
                    SymbolKind::Function,
                    file_path,
                    node.start_position().row + 1,
                    python_visibility(name),
                    symbol_signature(source, node.start_position().row + 1, 120),
                );
                sym.end_line = Some(node.end_position().row + 1);
                facts.symbols.push(sym);
            }
            // Walk body to capture call sites inside functions
            if let Some(body) = node.child_by_field_name("body") {
                for child in named_children(body) {
                    walk_python_node(child, file_path, source, lookup, facts);
                }
            }
        }
        "call" => {
            extract_python_call_site(node, file_path, source, facts);
            // still walk args/body for nested calls
            for child in named_children(node) {
                walk_python_node(child, file_path, source, lookup, facts);
            }
        }
        _ => {
            for child in named_children(node) {
                walk_python_node(child, file_path, source, lookup, facts);
            }
        }
    }
}

const PYTHON_BUILTIN_BLOCKLIST: &[&str] = &[
    "len", "str", "int", "float", "bool", "list", "dict", "set", "tuple", "type", "print",
    "range", "enumerate", "zip", "map", "filter", "sorted", "reversed", "sum", "min", "max",
    "abs", "round", "repr", "format", "isinstance", "issubclass", "hasattr", "getattr",
    "setattr", "delattr", "super", "object", "property", "classmethod", "staticmethod",
    "open", "input", "vars", "dir", "id", "hash", "next", "iter", "any", "all",
    "callable", "chr", "ord", "hex", "oct", "bin", "bytes", "bytearray", "memoryview",
    "exec", "eval", "compile", "globals", "locals", "help", "exit", "quit",
    "append", "extend", "insert", "remove", "pop", "clear", "copy", "keys", "values",
    "items", "get", "update", "add", "discard", "union", "intersection", "difference",
    "split", "join", "strip", "lstrip", "rstrip", "replace", "startswith", "endswith",
    "find", "index", "count", "upper", "lower", "title", "encode", "decode", "format_map",
];

fn extract_python_call_site(node: Node, file_path: &str, source: &str, facts: &mut FileFacts) {
    let Some(func_node) = node.child_by_field_name("function") else {
        return;
    };
    let name = match func_node.kind() {
        "identifier" => func_node.utf8_text(source.as_bytes()).ok().map(str::to_string),
        "attribute" => func_node
            .child_by_field_name("attribute")
            .and_then(|n| n.utf8_text(source.as_bytes()).ok())
            .map(str::to_string),
        _ => None,
    };
    let Some(name) = name else { return };
    if name.len() < 3 || PYTHON_BUILTIN_BLOCKLIST.contains(&name.as_str()) {
        return;
    }
    let line = node.start_position().row + 1;
    facts
        .symbol_references
        .push(call_site(&name, file_path, line));
}

fn walk_python_class_body(
    node: Node,
    class_name: &str,
    file_path: &str,
    source: &str,
    lookup: &RepoLookup,
    facts: &mut FileFacts,
) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "function_definition" => {
                if let Some(name) = child
                    .child_by_field_name("name")
                    .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                {
                    let mut sym = symbol(
                        name.to_string(),
                        SymbolKind::Function,
                        file_path,
                        child.start_position().row + 1,
                        python_visibility(name),
                        symbol_signature(source, child.start_position().row + 1, 120),
                    );
                    sym.end_line = Some(child.end_position().row + 1);
                    sym.parent_class = Some(class_name.to_string());
                    facts.symbols.push(sym);
                }
                // Walk body for call sites
                if let Some(body) = child.child_by_field_name("body") {
                    for sub in named_children(body) {
                        walk_python_node(sub, file_path, source, lookup, facts);
                    }
                }
            }
            "decorated_definition" => {
                // Handles @staticmethod, @classmethod, @property, etc.
                let mut inner_cursor = child.walk();
                for inner in child.named_children(&mut inner_cursor) {
                    if inner.kind() == "function_definition" {
                        if let Some(name) = inner
                            .child_by_field_name("name")
                            .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                        {
                            let mut sym = symbol(
                                name.to_string(),
                                SymbolKind::Function,
                                file_path,
                                inner.start_position().row + 1,
                                python_visibility(name),
                                symbol_signature(source, inner.start_position().row + 1, 120),
                            );
                            sym.end_line = Some(inner.end_position().row + 1);
                            sym.parent_class = Some(class_name.to_string());
                            facts.symbols.push(sym);
                        }
                        // Walk body for call sites
                        if let Some(body) = inner.child_by_field_name("body") {
                            for sub in named_children(body) {
                                walk_python_node(sub, file_path, source, lookup, facts);
                            }
                        }
                    }
                }
            }
            "class_definition" => {
                // Nested class — push it and walk its body too
                if let Some(name) = child
                    .child_by_field_name("name")
                    .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                {
                    let mut sym = symbol(
                        name.to_string(),
                        SymbolKind::Struct,
                        file_path,
                        child.start_position().row + 1,
                        python_visibility(name),
                        symbol_signature(source, child.start_position().row + 1, 120),
                    );
                    sym.end_line = Some(child.end_position().row + 1);
                    facts.symbols.push(sym);
                    if let Some(body) = child.child_by_field_name("body") {
                        walk_python_class_body(body, name, file_path, source, lookup, facts);
                    }
                }
            }
            _ => {}
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
                ..Default::default()
            })
            .collect::<Vec<_>>();
        RepoLookup::new(&files)
    }

    #[test]
    fn extracts_class_methods() {
        let source = r#"
class RequestContext:
    def push(self):
        pass

    def pop(self):
        pass

    @staticmethod
    def get_current():
        pass

    @classmethod
    def create(cls, app):
        pass
"#;
        let facts = extract_file_facts("app/ctx.py", source, &lookup(&["app/ctx.py"]));
        let names: Vec<_> = facts.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"RequestContext"), "class itself");
        assert!(names.contains(&"push"), "instance method");
        assert!(names.contains(&"pop"), "instance method");
        assert!(names.contains(&"get_current"), "@staticmethod");
        assert!(names.contains(&"create"), "@classmethod");
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

    #[test]
    fn extracts_call_sites() {
        let source = r#"
from app.ctx import RequestContext

def setup():
    ctx = RequestContext()
    ctx.push()
    result = process_data(ctx)
    print(result)
    len(result)
"#;
        let facts = extract_file_facts("app/setup.py", source, &lookup(&["app/setup.py"]));
        let call_names: Vec<_> = facts
            .symbol_references
            .iter()
            .filter(|b| b.reason == "call site")
            .map(|b| b.symbol_name.as_str())
            .collect();
        assert!(call_names.contains(&"RequestContext"), "constructor call");
        assert!(call_names.contains(&"push"), "method call");
        assert!(call_names.contains(&"process_data"), "function call");
        // builtins must be excluded
        assert!(!call_names.contains(&"print"), "print is a builtin");
        assert!(!call_names.contains(&"len"), "len is a builtin");
    }
}
