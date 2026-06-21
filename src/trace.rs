use anyhow::Result;
use std::path::Path;

use crate::dep_resolve::resolve_dep_for_symbol;
use crate::index::{EdgeConfidence, RepoIndex};
use crate::types::{TraceCallSite, TraceDefinition, TraceReport};

pub fn build_report(
    symbol_name: &str,
    index: &RepoIndex,
    repo_root: &Path,
) -> Result<TraceReport> {
    // 1. Definitions: symbols whose name matches (case-insensitive).
    let defined_in: Vec<TraceDefinition> = index
        .symbols
        .iter()
        .filter(|s| s.name.eq_ignore_ascii_case(symbol_name))
        .map(|s| TraceDefinition {
            file: s.file_path.clone(),
            line: s.line_number,
            kind: s.kind.to_string(),
            parent_class: s.parent_class.clone(),
        })
        .collect();

    // Symbol not found locally — run dep detection path.
    if defined_in.is_empty() {
        return build_external_report(symbol_name, index, repo_root);
    }

    // 2. Callers: symbol_references where symbol_name matches and target_file is known.
    let mut callers: Vec<TraceCallSite> = index
        .symbol_references
        .iter()
        .filter(|r| {
            r.symbol_name.eq_ignore_ascii_case(symbol_name)
                && r.target_file.is_some()
                && r.confidence != EdgeConfidence::Ambiguous
        })
        .map(|r| TraceCallSite {
            file: r.from_file.clone(),
            line: r.line_number,
            context: r.context.to_string(),
            confidence: r.confidence.to_string(),
        })
        .collect();

    // Also include inferred call-site refs (target_file=None).
    let mut call_sites: Vec<TraceCallSite> = index
        .symbol_references
        .iter()
        .filter(|r| {
            r.symbol_name.eq_ignore_ascii_case(symbol_name)
                && r.target_file.is_none()
                && r.confidence == EdgeConfidence::Inferred
        })
        .map(|r| TraceCallSite {
            file: r.from_file.clone(),
            line: r.line_number,
            context: r.context.to_string(),
            confidence: r.confidence.to_string(),
        })
        .collect();

    callers.sort_by(|a, b| a.file.cmp(&b.file).then(a.line.cmp(&b.line)));
    callers.dedup_by(|a, b| a.file == b.file && a.line == b.line);
    call_sites.sort_by(|a, b| a.file.cmp(&b.file).then(a.line.cmp(&b.line)));
    call_sites.dedup_by(|a, b| a.file == b.file && a.line == b.line);

    for cs in call_sites {
        if !callers.iter().any(|c| c.file == cs.file && c.line == cs.line) {
            callers.push(cs);
        }
    }
    callers.sort_by(|a, b| {
        let a_prod = (a.context != "production") as u8;
        let b_prod = (b.context != "production") as u8;
        a_prod.cmp(&b_prod).then(a.file.cmp(&b.file))
    });

    // 3. Callees: symbols referenced from the definition files, pointing elsewhere.
    let def_files: Vec<&str> = defined_in.iter().map(|d| d.file.as_str()).collect();
    let mut callees: Vec<TraceCallSite> = index
        .symbol_references
        .iter()
        .filter(|r| {
            def_files.contains(&r.from_file.as_str())
                && r.target_file
                    .as_deref()
                    .map(|t| !def_files.contains(&t))
                    .unwrap_or(false)
                && r.confidence == EdgeConfidence::Extracted
        })
        .map(|r| TraceCallSite {
            file: r
                .target_file
                .clone()
                .unwrap_or_else(|| r.from_file.clone()),
            line: r.target_line.unwrap_or(r.line_number),
            context: format!("via {}", r.symbol_name),
            confidence: r.confidence.to_string(),
        })
        .collect();
    callees.sort_by(|a, b| a.file.cmp(&b.file).then(a.line.cmp(&b.line)));
    callees.dedup_by(|a, b| a.file == b.file && a.context == b.context);
    callees.truncate(20);

    let total = callers.len() + callees.len();
    let note = if total == 0 {
        Some(
            "No caller/callee data found. The index may not have captured references for this \
             symbol. Try rebuilding with `agentgrep index`."
                .to_string(),
        )
    } else {
        None
    };

    let mut next_actions = Vec::new();
    for def in &defined_in {
        next_actions.push(format!(
            "agentgrep peek {} --file {}",
            symbol_name, def.file
        ));
    }
    for caller in callers.iter().take(3) {
        next_actions.push(format!("open {}:{}", caller.file, caller.line));
    }

    Ok(TraceReport {
        symbol: symbol_name.to_string(),
        index_status: "found".to_string(),
        defined_in,
        callers,
        callees,
        next_actions,
        dep_package: None,
        note,
    })
}

/// Build a TraceReport for a symbol not found in the local index.
/// Tries to identify which external dep provides it via:
/// 1. Direct dep_imports table lookup (index-time explicit import records)
/// 2. query-time AST scan of call site files (type annotations on receivers)
fn build_external_report(
    symbol_name: &str,
    index: &RepoIndex,
    repo_root: &Path,
) -> Result<TraceReport> {
    // Gather every reference to this symbol from the index.
    let references: Vec<&crate::index::IndexedSymbolReference> = index
        .symbol_references
        .iter()
        .filter(|r| r.symbol_name.eq_ignore_ascii_case(symbol_name))
        .collect();

    // Layer 1: exact name match in dep_imports (explicit import statement).
    let dep_package = index
        .dep_imports
        .iter()
        .find(|d| d.symbol_or_module.eq_ignore_ascii_case(symbol_name))
        .map(|d| d.dep_package.clone())
        // Layer 2: AST scan of call site files for type annotations.
        .or_else(|| {
            if !references.is_empty() {
                resolve_dep_for_symbol(symbol_name, index, repo_root)
            } else {
                None
            }
        });

    let (index_status, note) = match &dep_package {
        Some(pkg) => (
            "external".to_string(),
            Some(format!(
                "'{symbol_name}' is not defined in this repo — from external dep '{pkg}'. \
                 Check that dep's documentation or source."
            )),
        ),
        None if references.is_empty() => (
            "not_found".to_string(),
            Some(format!(
                "'{symbol_name}' not found in this repo and has no recorded references. \
                 Check spelling, or use `rg '{symbol_name}'` for a raw search."
            )),
        ),
        None => (
            "external".to_string(),
            Some(format!(
                "'{symbol_name}' is not defined in this repo ({} reference(s) found). \
                 Likely from an external dependency — check the import statements in the \
                 files below, or run `rg '{symbol_name}'` for a raw search.",
                references.len()
            )),
        ),
    };

    // Build a caller list from references so the agent can see context.
    let mut callers: Vec<TraceCallSite> = references
        .iter()
        .map(|r| TraceCallSite {
            file: r.from_file.clone(),
            line: r.line_number,
            context: r.context.to_string(),
            confidence: r.confidence.to_string(),
        })
        .collect();
    callers.sort_by(|a, b| {
        let a_prod = (a.context != "production") as u8;
        let b_prod = (b.context != "production") as u8;
        a_prod.cmp(&b_prod).then(a.file.cmp(&b.file))
    });
    callers.dedup_by(|a, b| a.file == b.file && a.line == b.line);
    callers.truncate(15);

    let mut next_actions = Vec::new();
    if let Some(pkg) = &dep_package {
        next_actions.push(format!(
            "# Check external dep documentation for '{pkg}'"
        ));
    }
    for caller in callers.iter().take(3) {
        next_actions.push(format!("open {}:{}", caller.file, caller.line));
    }
    if dep_package.is_none() {
        next_actions.push(format!("rg '{symbol_name}' --type-add 'src:*.{{rs,py,ts,js}}' -t src"));
    }

    Ok(TraceReport {
        symbol: symbol_name.to_string(),
        index_status,
        defined_in: vec![],
        callers,
        callees: vec![],
        next_actions,
        dep_package,
        note,
    })
}

pub fn write_report(report: &TraceReport, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(report)?);
        return Ok(());
    }

    println!("Trace: {}", report.symbol);
    println!();

    match report.index_status.as_str() {
        "external" => {
            if let Some(pkg) = &report.dep_package {
                println!("Status: not in repo — from external dep '{pkg}'");
            } else {
                println!("Status: not in repo (external, package not determined)");
            }
            println!();
            if !report.callers.is_empty() {
                println!("Referenced in ({}):", report.callers.len());
                for c in &report.callers {
                    println!("  {}:{}  [{}, {}]", c.file, c.line, c.context, c.confidence);
                }
            }
        }
        "not_found" => {
            println!("Status: not found in repo or dep records");
        }
        _ => {
            println!("Defined in ({}):", report.defined_in.len());
            for d in &report.defined_in {
                let cls = d
                    .parent_class
                    .as_deref()
                    .map(|c| format!(" (class: {c})"))
                    .unwrap_or_default();
                println!("  {}:{}  [{}{}]", d.file, d.line, d.kind, cls);
            }

            println!();
            println!("Callers ({}):", report.callers.len());
            if report.callers.is_empty() {
                println!("  (none found in index)");
            }
            for c in &report.callers {
                println!("  {}:{}  [{}, {}]", c.file, c.line, c.context, c.confidence);
            }

            println!();
            println!("Callees from definition file ({}):", report.callees.len());
            if report.callees.is_empty() {
                println!("  (none found in index)");
            }
            for c in &report.callees {
                println!("  {}:{}  [{}]", c.file, c.line, c.context);
            }
        }
    }

    if let Some(note) = &report.note {
        println!();
        println!("Note: {note}");
    }

    if !report.next_actions.is_empty() {
        println!();
        println!("Next:");
        for action in &report.next_actions {
            println!("  {action}");
        }
    }

    Ok(())
}
