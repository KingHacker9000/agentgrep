use anyhow::Result;
use std::path::Path;

use crate::caller_body;
use crate::dep_resolve::resolve_dep_for_symbol;
use crate::index::{EdgeConfidence, ReferenceContext, RepoIndex};
use crate::types::{TraceCallSite, TraceDefinition, TraceReport};

pub fn build_report(
    symbol_name: &str,
    index: &RepoIndex,
    repo_root: &Path,
    callers_body: bool,
    include_tests: bool,
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

    if defined_in.is_empty() {
        return build_external_report(symbol_name, index, repo_root, callers_body, include_tests);
    }

    // 2. Collect all callers (resolved + inferred).
    let mut all_callers: Vec<TraceCallSite> = index
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
            containing_function: None,
        })
        .collect();

    let inferred: Vec<TraceCallSite> = index
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
            containing_function: None,
        })
        .collect();

    // Merge and dedup.
    for cs in inferred {
        if !all_callers
            .iter()
            .any(|c| c.file == cs.file && c.line == cs.line)
        {
            all_callers.push(cs);
        }
    }
    all_callers.sort_by(|a, b| a.file.cmp(&b.file).then(a.line.cmp(&b.line)));
    all_callers.dedup_by(|a, b| a.file == b.file && a.line == b.line);

    // Split production vs test callers.
    let (mut callers, mut test_callers) = if include_tests {
        let prod: Vec<_> = all_callers
            .iter()
            .filter(|c| c.context == "production")
            .cloned()
            .collect();
        let test: Vec<_> = all_callers
            .iter()
            .filter(|c| c.context != "production")
            .cloned()
            .collect();
        (prod, test)
    } else {
        // Keep existing behaviour: all callers together, production first.
        all_callers.sort_by(|a, b| {
            let a_prod = (a.context != "production") as u8;
            let b_prod = (b.context != "production") as u8;
            a_prod.cmp(&b_prod).then(a.file.cmp(&b.file))
        });
        (all_callers, vec![])
    };

    // 3. Callees from definition files.
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
            containing_function: None,
        })
        .collect();
    callees.sort_by(|a, b| a.file.cmp(&b.file).then(a.line.cmp(&b.line)));
    callees.dedup_by(|a, b| a.file == b.file && a.context == b.context);
    callees.truncate(20);

    // 4. Optionally enrich callers with AST-extracted containing function bodies.
    if callers_body {
        enrich_with_bodies(
            &mut callers,
            caller_body::MAX_CALLERS_WITH_BODY,
            repo_root,
        );
        if include_tests {
            enrich_with_bodies(
                &mut test_callers,
                caller_body::MAX_TEST_CALLERS_WITH_BODY,
                repo_root,
            );
        }
    }

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
        test_callers,
        next_actions,
        dep_package: None,
        note,
    })
}

fn enrich_with_bodies(callers: &mut Vec<TraceCallSite>, cap: usize, repo_root: &Path) {
    for caller in callers.iter_mut().take(cap) {
        caller.containing_function =
            caller_body::from_file(&caller.file, caller.line, repo_root);
    }
}

fn build_external_report(
    symbol_name: &str,
    index: &RepoIndex,
    repo_root: &Path,
    callers_body: bool,
    _include_tests: bool,
) -> Result<TraceReport> {
    let references: Vec<&crate::index::IndexedSymbolReference> = index
        .symbol_references
        .iter()
        .filter(|r| r.symbol_name.eq_ignore_ascii_case(symbol_name))
        .collect();

    let dep_package = index
        .dep_imports
        .iter()
        .find(|d| d.symbol_or_module.eq_ignore_ascii_case(symbol_name))
        .map(|d| d.dep_package.clone())
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
                 Likely from an external dependency — check import statements in the files \
                 below, or run `rg '{symbol_name}'` for a raw search.",
                references.len()
            )),
        ),
    };

    let mut callers: Vec<TraceCallSite> = references
        .iter()
        .filter(|r| r.context == ReferenceContext::Production)
        .map(|r| TraceCallSite {
            file: r.from_file.clone(),
            line: r.line_number,
            context: r.context.to_string(),
            confidence: r.confidence.to_string(),
            containing_function: None,
        })
        .collect();
    callers.sort_by(|a, b| a.file.cmp(&b.file).then(a.line.cmp(&b.line)));
    callers.dedup_by(|a, b| a.file == b.file && a.line == b.line);
    callers.truncate(15);

    if callers_body {
        enrich_with_bodies(
            &mut callers,
            caller_body::MAX_CALLERS_WITH_BODY,
            repo_root,
        );
    }

    let mut next_actions = Vec::new();
    if let Some(pkg) = &dep_package {
        next_actions.push(format!("# Check external dep documentation for '{pkg}'"));
    }
    for caller in callers.iter().take(3) {
        next_actions.push(format!("open {}:{}", caller.file, caller.line));
    }
    if dep_package.is_none() {
        next_actions.push(format!(
            "rg '{symbol_name}' --type-add 'src:*.{{rs,py,ts,js}}' -t src"
        ));
    }

    Ok(TraceReport {
        symbol: symbol_name.to_string(),
        index_status,
        defined_in: vec![],
        callers,
        callees: vec![],
        test_callers: vec![],
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
                    print!("  {}:{}  [{}, {}]", c.file, c.line, c.context, c.confidence);
                    if let Some(f) = &c.containing_function {
                        print!("  fn:{}", f.name);
                    }
                    println!();
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
                if let Some(f) = &c.containing_function {
                    let trunc = if f.truncated { " [truncated]" } else { "" };
                    println!("  fn: {} (lines {}-{}){}", f.name, f.line_start, f.line_end, trunc);
                    for line in f.body.lines().take(3) {
                        println!("    {line}");
                    }
                    if f.body.lines().count() > 3 {
                        println!("    ...");
                    }
                    println!();
                }
            }

            if !report.test_callers.is_empty() {
                println!();
                println!("Test callers ({}):", report.test_callers.len());
                for c in &report.test_callers {
                    println!("  {}:{}  [{}]", c.file, c.line, c.confidence);
                    if let Some(f) = &c.containing_function {
                        println!("  fn: {} (lines {}-{})", f.name, f.line_start, f.line_end);
                    }
                }
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
