use anyhow::{anyhow, Result};
use std::collections::{BTreeMap, BTreeSet};

use crate::index::{
    self, EdgeConfidence, FileRole, IndexedFile, IndexedSymbolReference, ReferenceContext,
};
use crate::map;
use crate::repo::RepoInfo;
use crate::symbol;
use crate::types::{
    Confidence, IndexedSymbol, MapEdge, RelatedFile, RelatedMode, RelatedReport, SymbolMatch,
    SymbolMatchMode,
};

const RELATED_EDGE_DISPLAY_LIMIT: usize = 5;
const RELATED_FILE_DISPLAY_LIMIT: usize = 5;
const RELATED_SYMBOL_DISPLAY_LIMIT: usize = 5;
const RELATED_REFERENCE_DISPLAY_LIMIT: usize = 5;

pub fn build_report(repo: &RepoInfo, input: &str) -> Result<RelatedReport> {
    let loaded = index::load(repo)?;
    build_report_from_loaded(repo, &loaded, input)
}

fn build_report_from_loaded(
    repo: &RepoInfo,
    loaded: &index::LoadedIndex,
    input: &str,
) -> Result<RelatedReport> {
    let query = input.trim().to_string();
    if query.is_empty() {
        return Err(anyhow!("related input must not be empty"));
    }

    let Some(index) = loaded.index.as_ref() else {
        let mode = guess_mode(&repo.root, &query);
        return Ok(build_missing_report(
            loaded.state.to_string(),
            &query,
            mode,
            repo,
        ));
    };

    let resolved_path = map::resolve_requested_path(&repo.root, &query);
    if let Some(file) = index.files.iter().find(|file| file.path == resolved_path) {
        return Ok(build_file_report(
            repo,
            &loaded.state.to_string(),
            index,
            file,
            &query,
        ));
    }

    let symbol_report = symbol::build_report_from_loaded(repo, loaded, &query)?;
    Ok(build_symbol_report(
        repo,
        &loaded.state.to_string(),
        index,
        symbol_report,
        &query,
    ))
}

pub fn write_report(report: &RelatedReport, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(report)?);
        return Ok(());
    }

    println!("Related query: {}", report.query);
    println!("- mode: {}", report.mode);
    println!("- index status: {}", report.index_status);
    if let Some(match_mode) = &report.match_mode {
        println!("- match mode: {}", match_mode);
    }
    if let Some(target_file) = &report.target_file {
        println!("- target file: {}", target_file);
    }
    if let Some(target_role) = &report.target_role {
        println!("- role: {}", target_role);
    }

    if !report.symbol_matches.is_empty() {
        println!(
            "Matched definitions ({} total):",
            report.symbol_matches.len()
        );
        for item in report
            .symbol_matches
            .iter()
            .take(RELATED_SYMBOL_DISPLAY_LIMIT)
        {
            render_symbol_match(item);
        }
        if report.symbol_matches.len() > RELATED_SYMBOL_DISPLAY_LIMIT {
            println!(
                "- ... showing {} of {}",
                RELATED_SYMBOL_DISPLAY_LIMIT,
                report.symbol_matches.len()
            );
        }
    }

    if !report.symbols.is_empty() {
        println!("Symbols ({} total):", report.symbols.len());
        render_symbols(&report.symbols);
    } else {
        println!("Symbols: none");
    }

    let focus_paths = report_focus_paths(report);
    render_edge_section("Outgoing", &report.edges, &focus_paths, true);
    render_edge_section("Incoming", &report.edges, &focus_paths, false);

    if !report.references.is_empty() {
        println!("References ({} total):", report.references.len());
        render_references(&report.references);
    } else {
        println!("References: none");
    }

    if !report.related_files.is_empty() {
        println!("Related files ({} total):", report.related_files.len());
        let max_score = report
            .related_files
            .iter()
            .map(|related| related.score)
            .fold(0.0_f64, f64::max);
        for related in report.related_files.iter().take(RELATED_FILE_DISPLAY_LIMIT) {
            render_related_file(report, related, &focus_paths, max_score);
        }
        if report.related_files.len() > RELATED_FILE_DISPLAY_LIMIT {
            println!(
                "- ... showing {} of {}",
                RELATED_FILE_DISPLAY_LIMIT,
                report.related_files.len()
            );
        }
    } else {
        println!("Related files: none");
    }

    println!("Next:");
    for action in &report.next_actions {
        println!("- {action}");
    }

    if report.index_status == "missing" {
        println!();
        println!("Index missing. Run `agentgrep index` first.");
    } else if report.index_status == "unverifiable" {
        println!();
        println!("Index unverifiable because repo revision is unavailable.");
    }

    Ok(())
}

fn build_missing_report(
    index_status: String,
    query: &str,
    mode: RelatedMode,
    repo: &RepoInfo,
) -> RelatedReport {
    let mut next_actions = vec!["agentgrep index".to_string()];
    if repo.rev.is_some() {
        next_actions.push("agentgrep index --status".to_string());
    }
    if matches!(mode, RelatedMode::File) {
        next_actions.insert(0, format!("open {}", query));
    }

    RelatedReport {
        query: query.to_string(),
        mode,
        index_status,
        match_mode: None,
        target_file: None,
        target_role: None,
        symbol_matches: Vec::new(),
        related_files: Vec::new(),
        edges: Vec::new(),
        symbols: Vec::new(),
        references: Vec::new(),
        next_actions,
    }
}

fn build_file_report(
    repo: &RepoInfo,
    index_status: &str,
    index: &index::RepoIndex,
    file: &IndexedFile,
    query: &str,
) -> RelatedReport {
    let focus_paths = vec![file.path.clone()];
    let edges = collect_edges_for_paths(index, &focus_paths);
    let symbols = collect_symbols_for_paths(index, &focus_paths);
    let references = group_references(collect_file_references(index, &file.path));
    let related_files = rank_related_files(index, &focus_paths, &references, false, &edges);
    let mut next_actions = crate::map::build_next_actions(repo, file, index_status);
    if let Some(best_symbol) = best_file_symbol(&index.symbols, &file.path) {
        push_unique_action(
            &mut next_actions,
            format!("agentgrep symbol {}", best_symbol.name),
        );
    } else {
        push_unique_action(
            &mut next_actions,
            format!("agentgrep symbol {}", file_stem_like(&file.path)),
        );
    }

    RelatedReport {
        query: query.to_string(),
        mode: RelatedMode::File,
        index_status: index_status.to_string(),
        match_mode: None,
        target_file: Some(file.path.clone()),
        target_role: Some(file.role.to_string()),
        symbol_matches: Vec::new(),
        related_files,
        edges,
        symbols,
        references,
        next_actions,
    }
}

fn build_symbol_report(
    repo: &RepoInfo,
    index_status: &str,
    index: &index::RepoIndex,
    symbol_report: crate::types::SymbolReport,
    query: &str,
) -> RelatedReport {
    let focus_paths = symbol_report
        .matches
        .iter()
        .map(|item| item.symbol.file_path.clone())
        .collect::<Vec<_>>();
    let edges = collect_edges_for_paths(index, &focus_paths);
    let symbols = collect_symbols_for_paths(index, &focus_paths);
    let references = group_references(
        symbol_report
            .matches
            .iter()
            .flat_map(|item| item.used_by.iter().cloned())
            .collect::<Vec<_>>(),
    );
    let related_files = rank_related_files(index, &focus_paths, &references, true, &edges);
    let next_actions = build_symbol_next_actions(repo, &symbol_report);

    RelatedReport {
        query: query.to_string(),
        mode: RelatedMode::Symbol,
        index_status: index_status.to_string(),
        match_mode: Some(symbol_report.match_mode),
        target_file: None,
        target_role: None,
        symbol_matches: symbol_report.matches,
        related_files,
        edges,
        symbols,
        references,
        next_actions,
    }
}

fn build_symbol_next_actions(
    repo: &RepoInfo,
    symbol_report: &crate::types::SymbolReport,
) -> Vec<String> {
    let mut actions = symbol_report.next_actions.clone();
    if let Some(first) = symbol_report.matches.first() {
        push_unique_action(&mut actions, format!("open {}", first.symbol.file_path));
        push_unique_action(
            &mut actions,
            format!("agentgrep map {}", first.symbol.file_path),
        );
        if !matches!(symbol_report.match_mode, SymbolMatchMode::Exact)
            || symbol_report.matches.len() > 1
        {
            push_unique_action(
                &mut actions,
                format!("agentgrep symbol {}", first.symbol.name),
            );
        }
    }

    if repo.rev.is_none() {
        push_unique_action(&mut actions, "agentgrep index".to_string());
    }

    actions
}

fn render_symbol_match(item: &SymbolMatch) {
    println!(
        "- {} [{} {}] {}:{}",
        item.symbol.name,
        item.symbol.kind,
        item.symbol.visibility,
        item.symbol.file_path,
        item.symbol.line_number
    );
    if let Some(signature) = &item.symbol.signature {
        println!("  signature: {}", signature);
    }
    println!("  role: {}", item.file_role);
    render_reference_summary(&item.used_by);
}

fn render_symbols(symbols: &[IndexedSymbol]) {
    for symbol in symbols.iter().take(RELATED_SYMBOL_DISPLAY_LIMIT) {
        let mut details = format!(
            "{} [{} {}] line {}",
            symbol.name, symbol.kind, symbol.visibility, symbol.line_number
        );
        if let Some(signature) = &symbol.signature {
            details.push_str(": ");
            details.push_str(signature);
        }
        println!("- {details}");
    }
    if symbols.len() > RELATED_SYMBOL_DISPLAY_LIMIT {
        println!(
            "- ... showing {} of {}",
            RELATED_SYMBOL_DISPLAY_LIMIT,
            symbols.len()
        );
    }
}

fn render_related_file(
    report: &RelatedReport,
    related: &RelatedFile,
    focus_paths: &[String],
    max_score: f64,
) {
    let score = normalized_related_score(related.score, max_score);
    println!(
        "- {} [{}] score {:.2} confidence {}",
        related.path, related.role, score, related.confidence
    );
    let reasons = summarize_related_reasons(report, &related.path, focus_paths);
    if reasons.is_empty() {
        return;
    }

    let display_lines = related_reason_lines(&reasons, RELATED_FILE_DISPLAY_LIMIT);
    if let Some((first, rest)) = display_lines.split_first() {
        println!("  reasons: {}", first);
        for line in rest {
            println!("  - {}", line);
        }
    }
}

fn render_edge_section(title: &str, edges: &[MapEdge], focus_paths: &[String], outgoing: bool) {
    let focus_set = focus_paths.iter().collect::<BTreeSet<_>>();
    let filtered = edges
        .iter()
        .filter(|edge| match focus_set.is_empty() {
            true => false,
            false if outgoing => focus_set.contains(&edge.from),
            false => focus_set.contains(&edge.to),
        })
        .collect::<Vec<_>>();

    println!("{} ({} total):", title, filtered.len());
    if filtered.is_empty() {
        println!("- none");
        return;
    }

    for edge in filtered.iter().take(RELATED_EDGE_DISPLAY_LIMIT) {
        println!(
            "- {} -> {} [{}] {}",
            edge.from, edge.to, edge.edge_type, edge.reason
        );
    }
    if filtered.len() > RELATED_EDGE_DISPLAY_LIMIT {
        println!(
            "- ... showing {} of {}",
            RELATED_EDGE_DISPLAY_LIMIT,
            filtered.len()
        );
    }
}

fn render_references(references: &[IndexedSymbolReference]) {
    for reference in references.iter().take(RELATED_REFERENCE_DISPLAY_LIMIT) {
        let mut details = format!(
            "{}:{} [{} / {}] {}",
            reference.from_file,
            reference.line_number,
            reference.context,
            reference.confidence,
            reference.reason
        );
        if let Some(target_file) = &reference.target_file {
            details.push_str(" -> ");
            details.push_str(target_file);
        }
        if let Some(target_line) = reference.target_line {
            details.push(':');
            details.push_str(&target_line.to_string());
        }
        if reference.additional_count > 0 {
            details.push_str(&format!(" (+{} more)", reference.additional_count));
        }
        println!("- {details}");
    }
    if references.len() > RELATED_REFERENCE_DISPLAY_LIMIT {
        println!(
            "- ... showing {} of {}",
            RELATED_REFERENCE_DISPLAY_LIMIT,
            references.len()
        );
    }
}

fn render_reference_summary(references: &[IndexedSymbolReference]) {
    if references.is_empty() {
        println!("  Used by: none");
        return;
    }

    let total = references.iter().map(reference_total_count).sum::<usize>();
    let production = references
        .iter()
        .filter(|reference| matches!(reference.context, ReferenceContext::Production))
        .map(reference_total_count)
        .sum::<usize>();
    let fixture_test = references
        .iter()
        .filter(|reference| {
            matches!(
                reference.context,
                ReferenceContext::Fixture | ReferenceContext::Test
            )
        })
        .map(reference_total_count)
        .sum::<usize>();
    let unknown = references
        .iter()
        .filter(|reference| matches!(reference.context, ReferenceContext::Unknown))
        .map(reference_total_count)
        .sum::<usize>();

    print!(
        "  Used by ({} total; production {}; test/fixture {}",
        total, production, fixture_test
    );
    if unknown > 0 {
        print!("; unknown {}", unknown);
    }
    println!("):");

    for reference in references.iter().take(RELATED_REFERENCE_DISPLAY_LIMIT) {
        let mut details = format!(
            "{}:{} [{} / {}] {}",
            reference.from_file,
            reference.line_number,
            reference.context,
            reference.confidence,
            reference.reason
        );
        if let Some(target_file) = &reference.target_file {
            details.push_str(" -> ");
            details.push_str(target_file);
        }
        if let Some(target_line) = reference.target_line {
            details.push(':');
            details.push_str(&target_line.to_string());
        }
        if reference.additional_count > 0 {
            details.push_str(&format!(" (+{} more)", reference.additional_count));
        }
        println!("  - {details}");
    }
    if references.len() > RELATED_REFERENCE_DISPLAY_LIMIT {
        println!(
            "  - ... showing {} of {}",
            RELATED_REFERENCE_DISPLAY_LIMIT,
            references.len()
        );
    }
}

fn related_reason_lines(reasons: &[RelatedReasonSummary], limit: usize) -> Vec<String> {
    let mut lines = reasons
        .iter()
        .take(limit)
        .map(|reason| {
            if reason.count > 1 {
                format!("{} ×{}", reason.label, reason.count)
            } else {
                reason.label.clone()
            }
        })
        .collect::<Vec<_>>();
    if reasons.len() > limit {
        lines.push(format!(
            "... showing {} of {} reasons",
            limit,
            reasons.len()
        ));
    }
    lines
}

fn normalized_related_score(score: f64, max_score: f64) -> f64 {
    if max_score <= 0.0 {
        0.0
    } else {
        (score / max_score).min(1.0)
    }
}

#[derive(Clone)]
struct RelatedReasonSummary {
    label: String,
    count: usize,
    weight: f64,
}

fn summarize_related_reasons(
    report: &RelatedReport,
    related_path: &str,
    focus_paths: &[String],
) -> Vec<RelatedReasonSummary> {
    let focus_set = focus_paths.iter().cloned().collect::<BTreeSet<_>>();
    let mut grouped: BTreeMap<String, RelatedReasonSummary> = BTreeMap::new();

    if report.mode == RelatedMode::Symbol
        && report
            .symbol_matches
            .iter()
            .any(|item| item.symbol.file_path == related_path)
    {
        insert_reason_summary(&mut grouped, "defines matched symbol".to_string(), 1000.0);
    }

    for edge in &report.edges {
        let connects_focus = focus_set.contains(&edge.from) || focus_set.contains(&edge.to);
        let touches_related = edge.from == related_path || edge.to == related_path;
        if !connects_focus || !touches_related {
            continue;
        }

        let label = if edge.edge_type == "same_area" {
            "same_area".to_string()
        } else {
            format!("{}: {}", edge.edge_type, edge.reason)
        };
        let weight = edge_weight(&edge.edge_type, &edge.confidence);
        insert_reason_summary(&mut grouped, label, weight);
    }

    for reference in &report.references {
        let touches_related = reference.target_file.as_deref() == Some(related_path)
            || reference.from_file == related_path;
        let touches_focus = focus_set.contains(&reference.from_file)
            || reference
                .target_file
                .as_ref()
                .map(|target| focus_set.contains(target))
                .unwrap_or(false);
        if !touches_related || !touches_focus {
            continue;
        }

        let context = reference.context.to_string();
        let label = format!("{} reference ({})", reference.symbol_name, context);
        let weight = reference_weight(&reference.context, &reference.confidence);
        insert_reason_summary(&mut grouped, label, weight);
    }

    let mut reasons = grouped.into_values().collect::<Vec<_>>();
    reasons.sort_by(|left, right| {
        right
            .weight
            .partial_cmp(&left.weight)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.label.cmp(&right.label))
    });
    reasons
}

fn insert_reason_summary(
    grouped: &mut BTreeMap<String, RelatedReasonSummary>,
    label: String,
    weight: f64,
) {
    grouped
        .entry(label.clone())
        .and_modify(|existing| {
            existing.count += 1;
            existing.weight = existing.weight.max(weight);
        })
        .or_insert(RelatedReasonSummary {
            label,
            count: 1,
            weight,
        });
}

fn collect_file_references(
    index: &index::RepoIndex,
    target_file: &str,
) -> Vec<IndexedSymbolReference> {
    index
        .symbol_references
        .iter()
        .filter(|reference| {
            reference.from_file == target_file
                || reference.target_file.as_deref() == Some(target_file)
        })
        .cloned()
        .collect::<Vec<_>>()
}

fn collect_edges_for_paths(index: &index::RepoIndex, focus_paths: &[String]) -> Vec<MapEdge> {
    let focus_set = focus_paths.iter().cloned().collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    let mut edges = Vec::new();

    for edge in index
        .edges
        .iter()
        .filter(|edge| focus_set.contains(&edge.from) || focus_set.contains(&edge.to))
    {
        let key = (
            edge.edge_type.clone(),
            edge.from.clone(),
            edge.to.clone(),
            edge.reason.clone(),
        );
        if seen.insert(key) {
            edges.push(edge);
        }
    }

    map::ordered_edges(&edges)
        .iter()
        .map(|edge| MapEdge {
            edge_type: edge.edge_type.clone(),
            from: edge.from.clone(),
            to: edge.to.clone(),
            confidence: edge.confidence.to_string(),
            reason: edge.reason.clone(),
        })
        .collect()
}

fn collect_symbols_for_paths(
    index: &index::RepoIndex,
    focus_paths: &[String],
) -> Vec<IndexedSymbol> {
    let focus_set = focus_paths.iter().cloned().collect::<BTreeSet<_>>();
    let mut symbols = index
        .symbols
        .iter()
        .filter(|symbol| focus_set.contains(&symbol.file_path))
        .cloned()
        .collect::<Vec<_>>();
    symbols.sort_by(|left, right| {
        left.file_path
            .cmp(&right.file_path)
            .then_with(|| left.line_number.cmp(&right.line_number))
            .then_with(|| left.name.cmp(&right.name))
    });
    symbols.truncate(RELATED_SYMBOL_DISPLAY_LIMIT);
    symbols
}

fn best_file_symbol<'a>(
    symbols: &'a [IndexedSymbol],
    file_path: &str,
) -> Option<&'a IndexedSymbol> {
    symbols
        .iter()
        .filter(|symbol| symbol.file_path == file_path)
        .min_by(|left, right| {
            symbol_priority(left)
                .cmp(&symbol_priority(right))
                .then_with(|| left.line_number.cmp(&right.line_number))
                .then_with(|| left.name.cmp(&right.name))
        })
}

fn symbol_priority(symbol: &IndexedSymbol) -> usize {
    let kind_rank = match symbol.kind {
        crate::types::SymbolKind::Struct => 0,
        crate::types::SymbolKind::Enum => 1,
        crate::types::SymbolKind::Trait => 2,
        crate::types::SymbolKind::TypeAlias => 3,
        crate::types::SymbolKind::Function => 4,
        crate::types::SymbolKind::Module => 5,
        crate::types::SymbolKind::Const => 6,
        crate::types::SymbolKind::Static => 7,
        crate::types::SymbolKind::Impl => 8,
        crate::types::SymbolKind::Unknown => 9,
    };
    let visibility_boost = match symbol.visibility {
        crate::types::Visibility::Public => 0,
        crate::types::Visibility::Private => 100,
    };
    kind_rank * 10 + visibility_boost
}

fn rank_related_files(
    index: &index::RepoIndex,
    focus_paths: &[String],
    references: &[IndexedSymbolReference],
    include_focus: bool,
    edges: &[MapEdge],
) -> Vec<RelatedFile> {
    let focus_set = focus_paths.iter().cloned().collect::<BTreeSet<_>>();
    let mut candidates: BTreeMap<String, RelatedAccum> = BTreeMap::new();

    if include_focus {
        for focus_path in focus_paths {
            add_related_candidate(
                &mut candidates,
                index,
                focus_path,
                1000.0,
                Confidence::High,
                "defines matched symbol".to_string(),
            );
        }
    }

    for edge in edges {
        let other = if focus_set.contains(&edge.from) {
            &edge.to
        } else {
            &edge.from
        };
        if !include_focus && focus_set.contains(other) {
            continue;
        }
        if other.is_empty() {
            continue;
        }
        let weight = edge_weight(&edge.edge_type, &edge.confidence);
        add_related_candidate(
            &mut candidates,
            index,
            other,
            weight,
            confidence_from_weight(weight),
            format!("{}: {}", edge.edge_type, edge.reason),
        );
    }

    for reference in references {
        let other = if focus_set.contains(&reference.from_file) {
            reference.target_file.as_deref()
        } else if reference
            .target_file
            .as_ref()
            .map(|target| focus_set.contains(target))
            .unwrap_or(false)
        {
            Some(reference.from_file.as_str())
        } else {
            None
        };

        let Some(other) = other else {
            continue;
        };
        if !include_focus && focus_set.contains(other) {
            continue;
        }

        let weight = reference_weight(&reference.context, &reference.confidence);
        add_related_candidate(
            &mut candidates,
            index,
            other,
            weight,
            confidence_from_weight(weight),
            format!(
                "{} reference for {}",
                reference.symbol_name, reference.reason
            ),
        );
    }

    let mut related = candidates
        .into_values()
        .map(|candidate| candidate.into_related_file())
        .collect::<Vec<_>>();
    related.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                confidence_rank(&right.confidence).cmp(&confidence_rank(&left.confidence))
            })
            .then_with(|| left.path.cmp(&right.path))
    });
    related.truncate(RELATED_FILE_DISPLAY_LIMIT);
    related
}

fn add_related_candidate(
    candidates: &mut BTreeMap<String, RelatedAccum>,
    index: &index::RepoIndex,
    path: &str,
    score: f64,
    confidence: Confidence,
    reason: String,
) {
    let role = file_role_for_path(index, path);
    let entry = candidates
        .entry(path.to_string())
        .or_insert_with(|| RelatedAccum::new(path.to_string(), role));
    entry.score += score;
    entry.confidence = max_confidence(entry.confidence.clone(), confidence);
    entry.reasons.insert(reason);
}

fn file_role_for_path(index: &index::RepoIndex, path: &str) -> String {
    index
        .files
        .iter()
        .find(|file| file.path == path)
        .map(|file| file.role.to_string())
        .unwrap_or_else(|| FileRole::Other.to_string())
}

fn edge_weight(edge_type: &str, confidence: &str) -> f64 {
    let base = match edge_type {
        "imports" | "references" | "declares_module" | "configures" => 100.0,
        "likely_test_for" => 75.0,
        "same_area" => 20.0,
        _ => 40.0,
    };
    base + edge_confidence_bonus_text(confidence)
}

fn reference_weight(context: &ReferenceContext, confidence: &EdgeConfidence) -> f64 {
    let base = match context {
        ReferenceContext::Production => 90.0,
        ReferenceContext::Fixture => 60.0,
        ReferenceContext::Test => 55.0,
        ReferenceContext::Unknown => 25.0,
    };
    base + edge_confidence_bonus_enum(confidence)
}

fn edge_confidence_bonus_text(confidence: &str) -> f64 {
    match confidence {
        "extracted" => 10.0,
        "inferred" => 5.0,
        _ => 0.0,
    }
}

fn edge_confidence_bonus_enum(confidence: &EdgeConfidence) -> f64 {
    match confidence {
        EdgeConfidence::Extracted => 10.0,
        EdgeConfidence::Inferred => 5.0,
        EdgeConfidence::Ambiguous => 0.0,
    }
}

fn confidence_from_weight(score: f64) -> Confidence {
    if score >= 100.0 {
        Confidence::High
    } else if score >= 50.0 {
        Confidence::Medium
    } else {
        Confidence::Low
    }
}

fn confidence_rank(confidence: &Confidence) -> usize {
    match confidence {
        Confidence::High => 0,
        Confidence::Medium => 1,
        Confidence::Low => 2,
    }
}

fn max_confidence(left: Confidence, right: Confidence) -> Confidence {
    if confidence_rank(&right) < confidence_rank(&left) {
        right
    } else {
        left
    }
}

fn reference_total_count(reference: &IndexedSymbolReference) -> usize {
    reference.additional_count + 1
}

fn group_references(references: Vec<IndexedSymbolReference>) -> Vec<IndexedSymbolReference> {
    let mut grouped: BTreeMap<
        (
            String,
            String,
            Option<String>,
            ReferenceContext,
            EdgeConfidence,
            String,
        ),
        IndexedSymbolReference,
    > = BTreeMap::new();

    for reference in references {
        let key = (
            reference.from_file.clone(),
            reference.symbol_name.clone(),
            reference.target_file.clone(),
            reference.context,
            reference.confidence.clone(),
            reference.reason.clone(),
        );
        grouped
            .entry(key)
            .and_modify(|existing: &mut IndexedSymbolReference| {
                existing.additional_count += reference.additional_count + 1;
                if reference.line_number < existing.line_number {
                    existing.line_number = reference.line_number;
                }
                if existing.target_line.is_none() {
                    existing.target_line = reference.target_line;
                }
            })
            .or_insert(reference);
    }

    let mut grouped = grouped.into_values().collect::<Vec<_>>();
    grouped.sort_by(|left, right| {
        reference_context_rank(left.context)
            .cmp(&reference_context_rank(right.context))
            .then_with(|| {
                reference_confidence_rank(&left.confidence)
                    .cmp(&reference_confidence_rank(&right.confidence))
            })
            .then_with(|| left.from_file.cmp(&right.from_file))
            .then_with(|| left.line_number.cmp(&right.line_number))
            .then_with(|| left.symbol_name.cmp(&right.symbol_name))
    });
    grouped
}

fn reference_context_rank(context: ReferenceContext) -> usize {
    match context {
        ReferenceContext::Production => 0,
        ReferenceContext::Fixture => 1,
        ReferenceContext::Test => 2,
        ReferenceContext::Unknown => 3,
    }
}

fn reference_confidence_rank(confidence: &EdgeConfidence) -> usize {
    match confidence {
        EdgeConfidence::Extracted => 0,
        EdgeConfidence::Inferred => 1,
        EdgeConfidence::Ambiguous => 2,
    }
}

fn guess_mode(repo_root: &std::path::Path, input: &str) -> RelatedMode {
    let resolved = map::resolve_requested_path(repo_root, input);
    if input.contains('/')
        || input.contains('\\')
        || input.contains('.')
        || std::path::Path::new(&resolved).exists()
    {
        RelatedMode::File
    } else {
        RelatedMode::Symbol
    }
}

fn report_focus_paths(report: &RelatedReport) -> Vec<String> {
    if let Some(target_file) = &report.target_file {
        return vec![target_file.clone()];
    }

    let mut paths = report
        .symbol_matches
        .iter()
        .map(|item| item.symbol.file_path.clone())
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    paths
}

fn push_unique_action(actions: &mut Vec<String>, action: String) {
    if !actions.iter().any(|existing| existing == &action) {
        actions.push(action);
    }
}

fn file_stem_like(path: &str) -> String {
    std::path::Path::new(path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(|stem| stem.replace('_', " "))
        .unwrap_or_default()
}

struct RelatedAccum {
    path: String,
    role: String,
    score: f64,
    confidence: Confidence,
    reasons: BTreeSet<String>,
}

impl RelatedAccum {
    fn new(path: String, role: String) -> Self {
        Self {
            path,
            role,
            score: 0.0,
            confidence: Confidence::Low,
            reasons: BTreeSet::new(),
        }
    }

    fn into_related_file(self) -> RelatedFile {
        RelatedFile {
            path: self.path,
            role: self.role,
            score: self.score,
            confidence: self.confidence,
            reasons: self.reasons.into_iter().collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::{
        EdgeConfidence, FileRole, IndexState, IndexStats, IndexedEdge, IndexedFile,
        IndexedSymbolReference, ReferenceContext, RepoIndex, INDEX_SCHEMA_VERSION,
    };
    use crate::repo::RepoInfo;
    use std::path::PathBuf;

    fn repo() -> RepoInfo {
        RepoInfo {
            root: PathBuf::from("C:/repo"),
            rev: Some("abc".to_string()),
            git_dir: None,
        }
    }

    fn loaded_index() -> index::LoadedIndex {
        index::LoadedIndex {
            index_path: PathBuf::from("C:/repo/.agentgrep/index.json"),
            state: IndexState::Fresh,
            index: Some(RepoIndex {
                schema_version: INDEX_SCHEMA_VERSION,
                repo_root: "C:/repo".to_string(),
                repo_rev: Some("abc".to_string()),
                indexed_at_unix: 1,
                files: vec![
                    IndexedFile {
                        path: "src/search.rs".to_string(),
                        role: FileRole::Source,
                        size_bytes: Some(100),
                        modified_unix: Some(1),
                        content_hash: Some("aa".to_string()),
                        ..Default::default()
                    },
                    IndexedFile {
                        path: "src/types.rs".to_string(),
                        role: FileRole::Source,
                        size_bytes: Some(100),
                        modified_unix: Some(1),
                        content_hash: Some("bb".to_string()),
                        ..Default::default()
                    },
                    IndexedFile {
                        path: "src/symbol.rs".to_string(),
                        role: FileRole::Source,
                        size_bytes: Some(100),
                        modified_unix: Some(1),
                        content_hash: Some("cc".to_string()),
                        ..Default::default()
                    },
                    IndexedFile {
                        path: "src/main.rs".to_string(),
                        role: FileRole::Source,
                        size_bytes: Some(100),
                        modified_unix: Some(1),
                        content_hash: Some("dd".to_string()),
                        ..Default::default()
                    },
                ],
                symbols: vec![
                    crate::types::IndexedSymbol {
                        name: "SearchResult".to_string(),
                        kind: crate::types::SymbolKind::Struct,
                        file_path: "src/search.rs".to_string(),
                        line_number: 11,
                        visibility: crate::types::Visibility::Public,
                        signature: Some("pub struct SearchResult {".to_string()),
                        end_line: None,

            parent_class: None,                    },
                    crate::types::IndexedSymbol {
                        name: "FindReport".to_string(),
                        kind: crate::types::SymbolKind::Struct,
                        file_path: "src/types.rs".to_string(),
                        line_number: 5,
                        visibility: crate::types::Visibility::Public,
                        signature: Some("pub struct FindReport {".to_string()),
                        end_line: None,

            parent_class: None,                    },
                    crate::types::IndexedSymbol {
                        name: "SearchCoverage".to_string(),
                        kind: crate::types::SymbolKind::Struct,
                        file_path: "src/types.rs".to_string(),
                        line_number: 66,
                        visibility: crate::types::Visibility::Public,
                        signature: Some("pub struct SearchCoverage {".to_string()),
                        end_line: None,

            parent_class: None,                    },
                    crate::types::IndexedSymbol {
                        name: "SearchCoverage".to_string(),
                        kind: crate::types::SymbolKind::Impl,
                        file_path: "src/types.rs".to_string(),
                        line_number: 77,
                        visibility: crate::types::Visibility::Private,
                        signature: Some("impl SearchCoverage {".to_string()),
                        end_line: None,

            parent_class: None,                    },
                ],
                symbol_references: vec![
                    IndexedSymbolReference {
                        from_file: "src/main.rs".to_string(),
                        symbol_name: "SearchResult".to_string(),
                        target_file: Some("src/search.rs".to_string()),
                        target_line: Some(11),
                        line_number: 24,
                        confidence: EdgeConfidence::Inferred,
                        reason: "qualified or token reference".to_string(),
                        context: ReferenceContext::Production,
                        additional_count: 0,
                    },
                    IndexedSymbolReference {
                        from_file: "src/index.rs".to_string(),
                        symbol_name: "SearchResult".to_string(),
                        target_file: Some("src/search.rs".to_string()),
                        target_line: Some(11),
                        line_number: 2434,
                        confidence: EdgeConfidence::Inferred,
                        reason: "qualified or token reference".to_string(),
                        context: ReferenceContext::Test,
                        additional_count: 2,
                    },
                    IndexedSymbolReference {
                        from_file: "src/symbol.rs".to_string(),
                        symbol_name: "SearchResult".to_string(),
                        target_file: Some("src/search.rs".to_string()),
                        target_line: Some(11),
                        line_number: 651,
                        confidence: EdgeConfidence::Inferred,
                        reason: "qualified or token reference".to_string(),
                        context: ReferenceContext::Test,
                        additional_count: 6,
                    },
                    IndexedSymbolReference {
                        from_file: "src/output.rs".to_string(),
                        symbol_name: "FindReport".to_string(),
                        target_file: Some("src/types.rs".to_string()),
                        target_line: Some(5),
                        line_number: 3,
                        confidence: EdgeConfidence::Extracted,
                        reason: "use statement reference".to_string(),
                        context: ReferenceContext::Production,
                        additional_count: 0,
                    },
                    IndexedSymbolReference {
                        from_file: "src/search.rs".to_string(),
                        symbol_name: "SearchCoverage".to_string(),
                        target_file: Some("src/types.rs".to_string()),
                        target_line: Some(66),
                        line_number: 6,
                        confidence: EdgeConfidence::Extracted,
                        reason: "use statement reference".to_string(),
                        context: ReferenceContext::Production,
                        additional_count: 0,
                    },
                ],
                edges: vec![
                    IndexedEdge {
                        edge_type: "imports".to_string(),
                        from: "src/search.rs".to_string(),
                        to: "src/types.rs".to_string(),
                        confidence: EdgeConfidence::Extracted,
                        reason: "imports crate::types".to_string(),
                    },
                    IndexedEdge {
                        edge_type: "same_area".to_string(),
                        from: "src/search.rs".to_string(),
                        to: "src/symbol.rs".to_string(),
                        confidence: EdgeConfidence::Extracted,
                        reason: "shared source area src".to_string(),
                    },
                    IndexedEdge {
                        edge_type: "declares_module".to_string(),
                        from: "src/main.rs".to_string(),
                        to: "src/search.rs".to_string(),
                        confidence: EdgeConfidence::Extracted,
                        reason: "declares module search".to_string(),
                    },
                ],
                stats: IndexStats {
                    file_count: 4,
                    role_counts: std::collections::BTreeMap::from([(FileRole::Source, 4)]),
                    symbol_count: 4,
                    symbol_kind_counts: std::collections::BTreeMap::new(),
                    symbol_reference_count: 5,
                    connection_count: 3,
                    ..Default::default()
                },
                        dep_imports: vec![],
            }),
        }
    }

    #[test]
    fn file_input_resolves_to_file_mode() {
        let report = build_report_from_loaded(&repo(), &loaded_index(), "src/search.rs").unwrap();
        assert_eq!(report.mode, RelatedMode::File);
        assert_eq!(report.target_file.as_deref(), Some("src/search.rs"));
        assert!(report.match_mode.is_none());
        assert!(report.symbol_matches.is_empty());
    }

    #[test]
    fn symbol_input_resolves_to_symbol_mode() {
        let report = build_report_from_loaded(&repo(), &loaded_index(), "SearchResult").unwrap();
        assert_eq!(report.mode, RelatedMode::Symbol);
        assert_eq!(report.match_mode, Some(SymbolMatchMode::Exact));
        assert!(report.target_file.is_none());
        assert!(!report.symbol_matches.is_empty());
    }

    #[test]
    fn missing_index_gives_useful_action() {
        let loaded = index::LoadedIndex {
            index_path: PathBuf::from("C:/repo/.agentgrep/index.json"),
            state: IndexState::Missing,
            index: None,
        };
        let report = build_report_from_loaded(&repo(), &loaded, "src/search.rs").unwrap();
        assert_eq!(report.index_status, "missing");
        assert!(report
            .next_actions
            .iter()
            .any(|action| action == "agentgrep index"));
    }

    #[test]
    fn related_files_prioritize_imports_over_same_area() {
        let report = build_report_from_loaded(&repo(), &loaded_index(), "src/search.rs").unwrap();
        assert_eq!(report.mode, RelatedMode::File);
        assert_eq!(report.related_files.first().unwrap().path, "src/types.rs");
    }

    #[test]
    fn file_mode_prefers_public_symbol_next_action() {
        let report = build_report_from_loaded(&repo(), &loaded_index(), "src/search.rs").unwrap();
        assert!(report
            .next_actions
            .iter()
            .any(|action| action == "agentgrep symbol SearchResult"));
        assert!(!report
            .next_actions
            .iter()
            .any(|action| action == "agentgrep symbol MATCH_LIMIT_PER_FILE"));
    }

    #[test]
    fn symbol_mode_includes_definition_and_used_by_context() {
        let report = build_report_from_loaded(&repo(), &loaded_index(), "SearchResult").unwrap();
        assert_eq!(report.mode, RelatedMode::Symbol);
        assert_eq!(report.related_files.first().unwrap().path, "src/search.rs");
        assert!(report
            .references
            .iter()
            .any(|reference| reference.from_file == "src/main.rs"));
    }

    #[test]
    fn related_reason_lines_caps_verbose_output() {
        let report = build_report_from_loaded(&repo(), &loaded_index(), "SearchResult").unwrap();
        let reasons =
            summarize_related_reasons(&report, "src/main.rs", &["src/search.rs".to_string()]);
        let lines = related_reason_lines(&reasons, RELATED_FILE_DISPLAY_LIMIT);
        assert!(lines.len() <= RELATED_FILE_DISPLAY_LIMIT + 1);
        assert!(!lines.is_empty());
    }

    #[test]
    fn json_includes_mode_related_files_and_next_actions() {
        let report = build_report_from_loaded(&repo(), &loaded_index(), "SearchResult").unwrap();
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["mode"], "symbol");
        assert!(json["related_files"].is_array());
        assert!(json["next_actions"].is_array());
    }
}

