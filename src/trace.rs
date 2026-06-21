use anyhow::{bail, Result};

use crate::index::{EdgeConfidence, RepoIndex};
use crate::types::{TraceCallSite, TraceDefinition, TraceReport};

pub fn build_report(symbol_name: &str, index: &RepoIndex) -> Result<TraceReport> {
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
        bail!(
            "symbol '{}' not found in index — run `agentgrep index` first",
            symbol_name
        );
    }

    // 2. Callers: symbol_references where symbol_name matches and target_file is known.
    //    These are "use" / import-level references — files that reference this symbol.
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

    // Also include call-site refs (target_file=None, Inferred) — these record where
    // the name appears as a call in the source but the target isn't resolved. Useful
    // for finding all usage sites.
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

    // Deduplicate by (file, line)
    callers.sort_by(|a, b| a.file.cmp(&b.file).then(a.line.cmp(&b.line)));
    callers.dedup_by(|a, b| a.file == b.file && a.line == b.line);
    call_sites.sort_by(|a, b| a.file.cmp(&b.file).then(a.line.cmp(&b.line)));
    call_sites.dedup_by(|a, b| a.file == b.file && a.line == b.line);

    // Merge call-sites into callers, avoiding duplicates
    for cs in call_sites {
        if !callers
            .iter()
            .any(|c| c.file == cs.file && c.line == cs.line)
        {
            callers.push(cs);
        }
    }
    callers.sort_by(|a, b| {
        // Production callers first
        let a_prod = (a.context != "production") as u8;
        let b_prod = (b.context != "production") as u8;
        a_prod.cmp(&b_prod).then(a.file.cmp(&b.file))
    });

    // 3. Callees: symbols that appear in the same file as a definition AND have
    //    an extracted reference in that file pointing to another file.
    //    Approximation: collect symbol_references FROM the definition files
    //    where the reference is not self-referential.
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
        index_status: "fresh".to_string(),
        defined_in,
        callers,
        callees,
        next_actions,
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

    if let Some(note) = &report.note {
        println!();
        println!("Note: {note}");
    }

    Ok(())
}
