use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::Serialize;

use crate::index::RepoIndex;
use crate::types::{IndexedSymbol, SymbolKind};

#[derive(Debug, Serialize)]
pub struct PeekReport {
    pub symbol: String,
    pub file_path: String,
    pub line_number: usize,
    pub end_line: usize,
    pub kind: SymbolKind,
    pub signature: Option<String>,
    pub body: Vec<BodyLine>,
}

#[derive(Debug, Serialize)]
pub struct BodyLine {
    pub line: usize,
    pub text: String,
}

pub fn peek_symbol(
    symbol_name: &str,
    file_hint: Option<&str>,
    index: &RepoIndex,
    repo_root: &str,
) -> Result<PeekReport> {
    let matches: Vec<&IndexedSymbol> = index
        .symbols
        .iter()
        .filter(|s| s.name.eq_ignore_ascii_case(symbol_name))
        .collect();

    if matches.is_empty() {
        bail!(
            "symbol '{}' not found in index — run `agentgrep index` first",
            symbol_name
        );
    }

    let sym = if let Some(hint) = file_hint {
        matches
            .iter()
            .find(|s| s.file_path.contains(hint))
            .copied()
            .with_context(|| {
                format!(
                    "symbol '{}' not found in files matching '{}'; found in: {}",
                    symbol_name,
                    hint,
                    matches
                        .iter()
                        .map(|s| s.file_path.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })?
    } else if matches.len() == 1 {
        matches[0]
    } else {
        // Prefer exact case match, then definition with extent info, then first
        matches
            .iter()
            .find(|s| s.name == symbol_name)
            .or_else(|| matches.iter().find(|s| s.end_line.is_some()))
            .copied()
            .unwrap_or(matches[0])
    };

    let Some(end_line) = sym.end_line else {
        bail!(
            "symbol '{}' in {} has no extent information — rebuild the index with `agentgrep index`",
            symbol_name,
            sym.file_path
        );
    };

    let abs_path = Path::new(repo_root).join(&sym.file_path);
    let source = std::fs::read_to_string(&abs_path)
        .with_context(|| format!("could not read {}", abs_path.display()))?;

    let start_line = sym.line_number.saturating_sub(1); // 0-based
    let end_inclusive = end_line.min(source.lines().count()); // 1-based, inclusive

    let body: Vec<BodyLine> = source
        .lines()
        .enumerate()
        .skip(start_line)
        .take(end_inclusive.saturating_sub(start_line))
        .map(|(i, text)| BodyLine {
            line: i + 1,
            text: text.to_string(),
        })
        .collect();

    Ok(PeekReport {
        symbol: sym.name.clone(),
        file_path: sym.file_path.clone(),
        line_number: sym.line_number,
        end_line,
        kind: sym.kind.clone(),
        signature: sym.signature.clone(),
        body,
    })
}
