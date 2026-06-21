use std::path::Path;

use tree_sitter::Node;

use crate::index::EdgeConfidence;
use crate::types::{SymbolKind, Visibility};

use super::extracted::{call_site, edge, file_facts, symbol, FileFacts, ImportBinding};
use super::language::{join_relative, normalize_path, parent_dir, parse_source, RepoLookup};
use super::symbol_signature;

pub fn extract_file_facts(file_path: &str, source: &str, lookup: &RepoLookup) -> FileFacts {
    let Some(tree) = parse_source(tree_sitter_javascript::LANGUAGE.into(), source) else {
        return file_facts();
    };

    let root = tree.root_node();
    let mut facts = file_facts();
    let mut cursor = root.walk();
    for child in root.named_children(&mut cursor) {
        walk_js_node(child, file_path, source, lookup, false, &mut facts);
    }
    facts
}

fn walk_js_node(
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
                if let Some(target) = resolve_js_import(file_path, source, lookup, source_node) {
                    facts.edges.push(edge(
                        "imports",
                        file_path,
                        &target,
                        EdgeConfidence::Extracted,
                        format!("imports {}", normalize_path(&target)),
                    ));
                    for imported_name in js_named_imports(node, source) {
                        facts.symbol_references.push(ImportBinding {
                            from_file: file_path.to_string(),
                            symbol_name: imported_name,
                            target_file: Some(target.clone()),
                            line_number: node.start_position().row + 1,
                            confidence: EdgeConfidence::Extracted,
                            reason: format!(
                                "direct import binding from {}",
                                normalize_path(&target)
                            ),
                        });
                    }
                    if let Some(default_import) = js_default_import(node, source) {
                        facts.symbol_references.push(ImportBinding {
                            from_file: file_path.to_string(),
                            symbol_name: default_import,
                            target_file: Some(target.clone()),
                            line_number: node.start_position().row + 1,
                            confidence: EdgeConfidence::Extracted,
                            reason: format!(
                                "default import binding from {}",
                                normalize_path(&target)
                            ),
                        });
                    }
                }
            }
        }
        "export_statement" => {
            if let Some(source_node) = node.child_by_field_name("source") {
                if let Some(target) = resolve_js_import(file_path, source, lookup, source_node) {
                    facts.edges.push(edge(
                        "imports",
                        file_path,
                        &target,
                        EdgeConfidence::Extracted,
                        format!("imports {}", normalize_path(&target)),
                    ));
                    for imported_name in js_named_imports(node, source) {
                        facts.symbol_references.push(ImportBinding {
                            from_file: file_path.to_string(),
                            symbol_name: imported_name,
                            target_file: Some(target.clone()),
                            line_number: node.start_position().row + 1,
                            confidence: EdgeConfidence::Extracted,
                            reason: format!(
                                "direct import binding from {}",
                                normalize_path(&target)
                            ),
                        });
                    }
                }
            }

            for child in named_children(node) {
                walk_js_node(child, file_path, source, lookup, true, facts);
            }
        }
        "call_expression" => {
            if is_require_call(node, source) {
                if let Some(target_node) = first_string_argument(node) {
                    if let Some(target) = resolve_js_import(file_path, source, lookup, target_node)
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
            } else {
                extract_js_call_site(node, file_path, source, facts);
            }
        }
        "function_declaration" | "generator_function_declaration" => {
            if let Some(name) = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source.as_bytes()).ok())
            {
                let mut sym = symbol(
                    name.to_string(),
                    SymbolKind::Function,
                    file_path,
                    node.start_position().row + 1,
                    js_visibility(name, exported),
                    symbol_signature(source, node.start_position().row + 1, 120),
                );
                sym.end_line = Some(node.end_position().row + 1);
                facts.symbols.push(sym);
            }
        }
        "class_declaration" => {
            if let Some(name) = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source.as_bytes()).ok())
            {
                let mut sym = symbol(
                    name.to_string(),
                    SymbolKind::Struct,
                    file_path,
                    node.start_position().row + 1,
                    js_visibility(name, exported),
                    symbol_signature(source, node.start_position().row + 1, 120),
                );
                sym.end_line = Some(node.end_position().row + 1);
                facts.symbols.push(sym);
                if let Some(body) = node.child_by_field_name("body") {
                    walk_js_class_body(body, name, file_path, source, facts);
                }
            }
        }
        "lexical_declaration" | "variable_declaration" => {
            for child in named_children(node) {
                if child.kind() == "variable_declarator" {
                    extract_js_variable_symbol(child, file_path, source, exported, facts);
                }
            }
        }
        "expression_statement" => {
            if let Some(inner) = node.named_child(0) {
                if inner.kind() == "assignment_expression" {
                    extract_js_assignment_symbol(inner, file_path, source, exported, facts);
                }
            }
            // Still walk in case children have exports or nested declarations
            for child in named_children(node) {
                walk_js_node(child, file_path, source, lookup, exported, facts);
            }
        }
        _ => {
            for child in named_children(node) {
                walk_js_node(child, file_path, source, lookup, exported, facts);
            }
        }
    }
}

fn walk_js_class_body(
    node: Node,
    class_name: &str,
    file_path: &str,
    source: &str,
    facts: &mut FileFacts,
) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "method_definition" {
            if let Some(name) = child
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source.as_bytes()).ok())
            {
                if name == "constructor" {
                    continue;
                }
                let vis = if name.starts_with('_') || name.starts_with('#') {
                    Visibility::Private
                } else {
                    Visibility::Public
                };
                let mut sym = symbol(
                    name.to_string(),
                    SymbolKind::Function,
                    file_path,
                    child.start_position().row + 1,
                    vis,
                    symbol_signature(source, child.start_position().row + 1, 120),
                );
                sym.end_line = Some(child.end_position().row + 1);
                sym.parent_class = Some(class_name.to_string());
                facts.symbols.push(sym);
            }
        }
    }
}

fn extract_js_variable_symbol(
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

    let mut sym = symbol(
        name.to_string(),
        kind,
        file_path,
        node.start_position().row + 1,
        js_visibility(name, exported),
        symbol_signature(source, node.start_position().row + 1, 120),
    );
    sym.end_line = Some(node.end_position().row + 1);
    facts.symbols.push(sym);
}

/// Extracts a symbol from `proto.use = function use(fn) {}` style assignments.
/// The right-hand side must be a function_expression or arrow_function.
/// The symbol name is taken from: (1) the function's own `id` field if present,
/// else (2) the last property segment of the left-hand side (`app.use` → `use`).
fn extract_js_assignment_symbol(
    node: Node,
    file_path: &str,
    source: &str,
    exported: bool,
    facts: &mut FileFacts,
) {
    let Some(rhs) = node.child_by_field_name("right") else {
        return;
    };
    if !matches!(rhs.kind(), "function_expression" | "arrow_function") {
        return;
    }

    // Try to get the function's own name first (e.g. `function use(fn) {}`).
    let fn_name = rhs
        .child_by_field_name("name")
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
        .map(str::trim)
        .filter(|s| !s.is_empty());

    // Fall back to the last segment of the LHS (e.g. `app.use` or `proto.use`).
    let lhs_name = node
        .child_by_field_name("left")
        .and_then(|lhs| {
            if lhs.kind() == "member_expression" {
                lhs.child_by_field_name("property")
            } else {
                None
            }
        })
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let name = fn_name.or(lhs_name);
    let Some(name) = name else {
        return;
    };

    let mut sym = symbol(
        name.to_string(),
        SymbolKind::Function,
        file_path,
        node.start_position().row + 1,
        js_visibility(name, exported),
        symbol_signature(source, node.start_position().row + 1, 120),
    );
    sym.end_line = Some(node.end_position().row + 1);
    facts.symbols.push(sym);
}

const JS_BUILTIN_BLOCKLIST: &[&str] = &[
    "console", "log", "error", "warn", "info", "debug", "toString", "valueOf", "hasOwnProperty",
    "call", "apply", "bind", "then", "catch", "finally", "resolve", "reject", "push", "pop",
    "shift", "unshift", "slice", "splice", "map", "filter", "reduce", "find", "findIndex",
    "forEach", "includes", "indexOf", "some", "every", "join", "split", "replace", "trim",
    "startsWith", "endsWith", "substring", "slice", "parseInt", "parseFloat", "isNaN",
    "setTimeout", "setInterval", "clearTimeout", "clearInterval", "fetch", "require", "use",
    "get", "set", "delete", "has", "add", "clear", "keys", "values", "entries", "next",
    "assign", "keys", "values", "entries", "create", "defineProperty", "freeze",
];

fn extract_js_call_site(node: Node, file_path: &str, source: &str, facts: &mut FileFacts) {
    let Some(func_node) = node.child_by_field_name("function") else {
        return;
    };
    let name = match func_node.kind() {
        "identifier" => func_node.utf8_text(source.as_bytes()).ok().map(str::to_string),
        "member_expression" => func_node
            .child_by_field_name("property")
            .and_then(|n| n.utf8_text(source.as_bytes()).ok())
            .map(str::to_string),
        _ => None,
    };
    let Some(name) = name else { return };
    if name.len() < 3 || JS_BUILTIN_BLOCKLIST.contains(&name.as_str()) {
        return;
    }
    let line = node.start_position().row + 1;
    facts
        .symbol_references
        .push(call_site(&name, file_path, line));
}

fn resolve_js_import(
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
        for ext in [".js", ".jsx", ".ts", ".tsx"] {
            candidates.push(format!("{joined}{ext}"));
        }
        for ext in [".js", ".jsx", ".ts", ".tsx"] {
            candidates.push(format!("{joined}/index{ext}"));
        }
    }

    lookup.resolve_candidates(candidates)
}

fn js_named_imports(node: Node, source: &str) -> Vec<String> {
    let Some(text) = node.utf8_text(source.as_bytes()).ok() else {
        return Vec::new();
    };
    let Some(start) = text.find('{') else {
        return Vec::new();
    };
    let Some(end) = text[start + 1..].find('}') else {
        return Vec::new();
    };

    text[start + 1..start + 1 + end]
        .split(',')
        .filter_map(|part| {
            let mut item = part.trim();
            if item.is_empty() {
                return None;
            }
            if let Some((before_as, _)) = item.split_once(" as ") {
                item = before_as.trim();
            }
            if item.is_empty() || item == "*" || item == "default" {
                None
            } else {
                Some(item.to_string())
            }
        })
        .collect()
}

fn js_default_import(node: Node, source: &str) -> Option<String> {
    let text = node.utf8_text(source.as_bytes()).ok()?.trim();
    let clause = text.strip_prefix("import")?.trim();
    let clause = clause
        .split_once(" from ")
        .map(|(before, _)| before.trim())
        .unwrap_or(clause);
    if clause.starts_with('{') || clause.starts_with('*') {
        return None;
    }

    let default = clause.split(',').next()?.trim();
    if default.is_empty() || default == "type" {
        None
    } else {
        Some(default.trim_start_matches("type ").trim().to_string())
    }
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

fn js_visibility(name: &str, exported: bool) -> Visibility {
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
                ..Default::default()
            })
            .collect::<Vec<_>>();
        RepoLookup::new(&files)
    }

    #[test]
    fn extracts_js_symbols_and_import_edges() {
        let source = r#"
import { LLMClient } from "./llm_client";
export { shared } from "../shared.js";
const run = () => {};
export class Runner {}
"#;
        let facts = extract_file_facts(
            "src/app.js",
            source,
            &lookup(&["src/app.js", "src/llm_client.js", "shared.js"]),
        );
        assert!(facts.symbols.iter().any(|symbol| symbol.name == "run"));
        assert!(facts.symbols.iter().any(|symbol| symbol.name == "Runner"));
        assert!(facts
            .symbol_references
            .iter()
            .any(|binding| binding.symbol_name == "LLMClient"));
        assert!(facts.edges.iter().any(|edge| edge.to == "shared.js"));
    }

    #[test]
    fn extracts_prototype_assignment() {
        let source = r#"
app.use = function use(fn) {
    this.stack.push(fn);
};

proto.handle = function handle(req, res, out) {
    return this.router.handle(req, res, out);
};

const arrow = (x) => x;
"#;
        let facts =
            extract_file_facts("lib/application.js", source, &lookup(&["lib/application.js"]));
        let names: Vec<_> = facts.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"use"), "named fn expression: app.use = function use");
        assert!(names.contains(&"handle"), "lhs fallback: proto.handle");
        assert!(names.contains(&"arrow"), "plain const arrow unaffected");
    }

    #[test]
    fn extracts_default_import_binding_refs() {
        let source = r#"
import App from "./App";
"#;
        let facts = extract_file_facts(
            "src/main.js",
            source,
            &lookup(&["src/main.js", "src/App.js"]),
        );
        assert!(facts
            .symbol_references
            .iter()
            .any(|binding| binding.symbol_name == "App"
                && binding.target_file.as_deref() == Some("src/App.js")));
    }
}
