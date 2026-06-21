use std::path::Path;

use tree_sitter::Node;

use crate::parser::language::{detect_language, parse_source, LanguageKind};
use crate::types::ContainingFunction;

/// Lines of function header always included for long functions.
const HEADER_LINES: usize = 12;
/// Lines of context before/after the call site in long functions.
const CALL_CONTEXT: usize = 6;
/// Functions longer than this are truncated (header + call-site window).
const MAX_BODY_LINES: usize = 60;
/// Max callers to enrich with bodies (to bound response size).
pub const MAX_CALLERS_WITH_BODY: usize = 10;
pub const MAX_TEST_CALLERS_WITH_BODY: usize = 5;

/// Extract the innermost function/method body containing `target_line` (1-indexed).
/// Returns None if the language is unsupported or no enclosing function is found.
pub fn extract(file_path: &str, source: &str, target_line: usize) -> Option<ContainingFunction> {
    let lang_kind = detect_language(file_path)?;
    let ts_lang = ts_language(lang_kind)?;
    let tree = parse_source(ts_lang, source)?;
    let target_row = target_line.saturating_sub(1); // tree-sitter is 0-indexed
    find_innermost(tree.root_node(), source, target_row)
}

fn ts_language(kind: LanguageKind) -> Option<tree_sitter::Language> {
    Some(match kind {
        LanguageKind::Rust => tree_sitter_rust::LANGUAGE.into(),
        LanguageKind::Python => tree_sitter_python::LANGUAGE.into(),
        LanguageKind::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
        LanguageKind::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        LanguageKind::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        LanguageKind::Go => tree_sitter_go::LANGUAGE.into(),
    })
}

fn is_fn_node(kind: &str) -> bool {
    matches!(
        kind,
        // Rust
        "function_item"
        | "closure_expression"
        // Python
        | "function_definition"
        | "async_function_definition"
        // JS / TS
        | "function_declaration"
        | "function_expression"
        | "arrow_function"
        | "method_definition"
        // Go
        | "method_declaration"
    )
}

fn find_innermost(node: Node, source: &str, target_row: usize) -> Option<ContainingFunction> {
    let start = node.start_position().row;
    let end = node.end_position().row;

    if target_row < start || target_row > end {
        return None;
    }

    // Recurse into children first — innermost (deepest) match wins.
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if let Some(found) = find_innermost(child, source, target_row) {
            return Some(found);
        }
    }

    if is_fn_node(node.kind()) {
        return build(node, source, target_row);
    }

    None
}

fn build(node: Node, source: &str, target_row: usize) -> Option<ContainingFunction> {
    let line_start = node.start_position().row + 1; // 1-indexed
    let line_end = node.end_position().row + 1;

    let name = node
        .child_by_field_name("name")
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("<anonymous@{line_start}>"));

    let lines: Vec<&str> = source.lines().collect();

    let signature = lines
        .get(line_start.saturating_sub(1))
        .map(|l| l.trim().to_string())
        .unwrap_or_default();

    let total = (line_end + 1).saturating_sub(line_start);

    let (body, truncated) = if total <= MAX_BODY_LINES {
        let text = lines
            .get(line_start.saturating_sub(1)..line_end)
            .unwrap_or(&[])
            .join("\n");
        (text, false)
    } else {
        // header block
        let header_end_0 = (line_start - 1 + HEADER_LINES).min(line_end - 1);
        let header: Vec<&str> = lines
            .get(line_start.saturating_sub(1)..=header_end_0)
            .unwrap_or(&[])
            .to_vec();

        // call-site window (0-indexed bounds)
        let ctx_start_0 = target_row.saturating_sub(CALL_CONTEXT);
        let ctx_end_0 = (target_row + CALL_CONTEXT).min(line_end - 1);

        let mut parts: Vec<String> = header.iter().map(|l| l.to_string()).collect();

        if ctx_start_0 > header_end_0 + 1 {
            let skipped = ctx_start_0 - header_end_0 - 1;
            parts.push(format!("    // ... ({skipped} lines omitted)"));
        }

        let ctx: Vec<&str> = lines
            .get(ctx_start_0..=ctx_end_0)
            .unwrap_or(&[])
            .to_vec();
        parts.extend(ctx.iter().map(|l| l.to_string()));

        if ctx_end_0 < line_end - 1 {
            let skipped = line_end - 1 - ctx_end_0;
            parts.push(format!("    // ... ({skipped} lines omitted)"));
        }

        (parts.join("\n"), true)
    };

    Some(ContainingFunction {
        name,
        signature,
        line_start,
        line_end,
        body,
        truncated,
    })
}

/// Read a file and extract the containing function for a call site.
/// Returns None on any I/O or parse failure (callers degrade gracefully).
pub fn from_file(
    file_path: &str,
    target_line: usize,
    repo_root: &Path,
) -> Option<ContainingFunction> {
    let abs = repo_root.join(file_path);
    let source = std::fs::read_to_string(&abs).ok()?;
    extract(file_path, &source, target_line)
}
