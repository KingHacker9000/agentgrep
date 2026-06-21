use anyhow::{anyhow, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::index::{
    self, EdgeConfidence, FileRole, IndexedEdge, IndexedFile, IndexedSymbolReference,
    ReferenceContext,
};
use crate::map;
use crate::repo::RepoInfo;
use crate::symbol;
use crate::types::{
    BlastImpactContext, BlastImpactedFile, BlastMode, BlastReport, BlastRiskLevel, Confidence,
    IndexedSymbol, SymbolMatchMode,
};

const IMPACT_FILE_DISPLAY_LIMIT: usize = 5;
const REFERENCE_DISPLAY_LIMIT: usize = 5;
const SYMBOL_DISPLAY_LIMIT: usize = 5;
const INSPECTION_ORDER_DISPLAY_LIMIT: usize = 5;

#[derive(Clone, Copy, Default)]
struct ReferenceTotals {
    total_references: usize,
    production_files: usize,
    production_references: usize,
    test_fixture_files: usize,
    test_fixture_references: usize,
    unknown_files: usize,
    unknown_references: usize,
}

pub fn build_report(repo: &RepoInfo, input: &str) -> Result<BlastReport> {
    let loaded = index::load(repo)?;
    build_report_from_loaded(repo, &loaded, input)
}

pub(crate) fn build_report_from_loaded(
    repo: &RepoInfo,
    loaded: &index::LoadedIndex,
    input: &str,
) -> Result<BlastReport> {
    let query = input.trim().to_string();
    if query.is_empty() {
        return Err(anyhow!("blast input must not be empty"));
    }

    let index_status = loaded.state.to_string();
    let Some(index) = loaded.index.as_ref() else {
        return Ok(build_missing_report(repo, &query, &index_status));
    };

    let resolved_path = map::resolve_requested_path(&repo.root, &query);
    if let Some(file) = index.files.iter().find(|file| file.path == resolved_path) {
        return Ok(build_file_report(repo, index_status, index, file, &query));
    }

    let symbol_report = symbol::build_report_from_loaded(repo, loaded, &query)?;
    Ok(build_symbol_report(
        repo,
        index_status,
        index,
        symbol_report,
        &query,
    ))
}

pub fn write_report(report: &BlastReport, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(report)?);
        return Ok(());
    }

    let reference_totals = summarize_reference_totals(&report.references);
    let max_score = report
        .impacted_files
        .iter()
        .map(|file| file.score)
        .fold(0.0_f64, f64::max);

    println!("Blast query: {}", report.query);
    println!("- target: {}", report.query);
    println!("- mode: {}", report.mode);
    println!("- index status: {}", report.index_status);
    println!(
        "- risk: {} ({})",
        report.risk_level,
        report
            .risk_reasons
            .first()
            .map(String::as_str)
            .unwrap_or("no risk summary available")
    );

    if report.affected_symbols.is_empty() {
        println!("Affected symbols: none");
    } else {
        println!(
            "Affected symbols ({} total):",
            report.affected_symbols.len()
        );
        for symbol in report.affected_symbols.iter().take(SYMBOL_DISPLAY_LIMIT) {
            render_symbol(symbol);
        }
        if report.affected_symbols.len() > SYMBOL_DISPLAY_LIMIT {
            println!(
                "- ... showing {} of {}",
                SYMBOL_DISPLAY_LIMIT,
                report.affected_symbols.len()
            );
        }
    }

    render_reference_section(&report.mode, &report.references, reference_totals);
    render_impacted_files_section(&report.impacted_files, max_score);
    render_inspection_order(&report.suggested_inspection_order);

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

fn build_missing_report(repo: &RepoInfo, query: &str, index_status: &str) -> BlastReport {
    let mode = guess_mode(&repo.root, query);
    let mut next_actions = vec!["agentgrep index".to_string()];
    if repo.rev.is_some() {
        next_actions.push("agentgrep index --status".to_string());
    }
    if matches!(mode, BlastMode::File) {
        next_actions.insert(0, format!("open {}", query));
    }

    BlastReport {
        query: query.to_string(),
        mode,
        index_status: index_status.to_string(),
        risk_level: BlastRiskLevel::Low,
        risk_reasons: vec!["index missing; run `agentgrep index`".to_string()],
        impacted_files: Vec::new(),
        affected_symbols: Vec::new(),
        references: Vec::new(),
        suggested_inspection_order: Vec::new(),
        next_actions,
    }
}

fn build_file_report(
    repo: &RepoInfo,
    index_status: String,
    index: &index::RepoIndex,
    file: &IndexedFile,
    query: &str,
) -> BlastReport {
    let focus_paths = vec![file.path.clone()];
    let references = collect_references(index, &focus_paths);
    let edges = collect_edges(index, &focus_paths);
    let impacted_files = collect_impacted_files(index, &focus_paths, &references, &edges);
    let affected_symbols = collect_file_symbols(index, &file.path);
    let suggested_inspection_order = build_inspection_order(&focus_paths, &impacted_files);
    let risk = assess_risk(BlastMode::File, &impacted_files, &references, &index_status);
    let next_actions = build_next_actions_for_file(
        repo,
        query,
        &file.path,
        &affected_symbols,
        &index_status,
        &risk,
    );

    BlastReport {
        query: query.to_string(),
        mode: BlastMode::File,
        index_status,
        risk_level: risk.level,
        risk_reasons: risk.reasons,
        impacted_files,
        affected_symbols,
        references,
        suggested_inspection_order,
        next_actions,
    }
}

fn build_symbol_report(
    repo: &RepoInfo,
    index_status: String,
    index: &index::RepoIndex,
    symbol_report: crate::types::SymbolReport,
    query: &str,
) -> BlastReport {
    let focus_paths = symbol_report
        .matches
        .iter()
        .map(|item| item.symbol.file_path.clone())
        .collect::<Vec<_>>();
    let references = collect_symbol_report_references(&symbol_report.matches);
    let edges = collect_edges(index, &focus_paths);
    let impacted_files = collect_impacted_files(index, &focus_paths, &references, &edges);
    let affected_symbols = symbol_report
        .matches
        .iter()
        .map(|item| item.symbol.clone())
        .collect::<Vec<_>>();
    let suggested_inspection_order = build_inspection_order(&focus_paths, &impacted_files);
    let risk = assess_risk(
        BlastMode::Symbol,
        &impacted_files,
        &references,
        &index_status,
    );
    let next_actions = build_next_actions_for_symbol(
        repo,
        query,
        &symbol_report,
        &affected_symbols,
        &index_status,
    );

    BlastReport {
        query: query.to_string(),
        mode: BlastMode::Symbol,
        index_status,
        risk_level: risk.level,
        risk_reasons: risk.reasons,
        impacted_files,
        affected_symbols,
        references,
        suggested_inspection_order,
        next_actions,
    }
}

fn collect_file_symbols(index: &index::RepoIndex, file_path: &str) -> Vec<IndexedSymbol> {
    let mut symbols = index
        .symbols
        .iter()
        .filter(|symbol| {
            symbol.file_path == file_path
                && symbol.visibility == crate::types::Visibility::Public
                && is_blast_symbol_kind(&symbol.kind)
        })
        .cloned()
        .collect::<Vec<_>>();
    symbols.sort_by(|left, right| {
        symbol_priority(left)
            .cmp(&symbol_priority(right))
            .then_with(|| left.line_number.cmp(&right.line_number))
            .then_with(|| left.name.cmp(&right.name))
    });
    symbols
}

fn collect_symbol_report_references(
    matches: &[crate::types::SymbolMatch],
) -> Vec<IndexedSymbolReference> {
    let references = matches
        .iter()
        .flat_map(|item| item.used_by.iter().cloned())
        .collect::<Vec<_>>();
    group_references(references)
}

fn collect_references(
    index: &index::RepoIndex,
    focus_paths: &[String],
) -> Vec<IndexedSymbolReference> {
    let focus_set = focus_paths.iter().cloned().collect::<BTreeSet<_>>();
    let references = index
        .symbol_references
        .iter()
        .filter(|reference| {
            reference
                .target_file
                .as_ref()
                .map(|target| focus_set.contains(target))
                .unwrap_or(false)
                && !focus_set.contains(&reference.from_file)
        })
        .cloned()
        .collect::<Vec<_>>();
    group_references(references)
}

fn collect_edges(index: &index::RepoIndex, focus_paths: &[String]) -> Vec<IndexedEdge> {
    let focus_set = focus_paths.iter().cloned().collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    let mut edges = Vec::new();

    for edge in index.edges.iter().filter(|edge| {
        focus_set.contains(&edge.to)
            || (edge.edge_type == "same_area"
                && (focus_set.contains(&edge.from) || focus_set.contains(&edge.to)))
    }) {
        let key = (
            edge.edge_type.clone(),
            edge.from.clone(),
            edge.to.clone(),
            edge.reason.clone(),
        );
        if seen.insert(key) {
            edges.push(edge.clone());
        }
    }

    edges
}

fn collect_impacted_files(
    index: &index::RepoIndex,
    focus_paths: &[String],
    references: &[IndexedSymbolReference],
    edges: &[IndexedEdge],
) -> Vec<BlastImpactedFile> {
    let focus_set = focus_paths.iter().cloned().collect::<BTreeSet<_>>();
    let mut candidates: BTreeMap<String, BlastImpactAccum> = BTreeMap::new();

    for reference in references {
        let Some(target_file) = reference.target_file.as_ref() else {
            continue;
        };
        if !focus_set.contains(target_file) || focus_set.contains(&reference.from_file) {
            continue;
        }

        let weight = reference_weight(&reference.context, &reference.confidence);
        let confidence = confidence_from_weight(weight);
        let category = match reference.context {
            ReferenceContext::Production => BlastImpactCategory::Production,
            ReferenceContext::Test | ReferenceContext::Fixture => BlastImpactCategory::TestFixture,
            ReferenceContext::Unknown => BlastImpactCategory::Unknown,
        };
        add_candidate(
            &mut candidates,
            index,
            &reference.from_file,
            weight,
            confidence,
            category,
            format!(
                "{} reference ({})",
                reference.symbol_name, reference.context
            ),
        );
    }

    for edge in edges {
        let other = other_edge_file(edge, &focus_set);
        let Some(other) = other else {
            continue;
        };
        if focus_set.contains(other) {
            continue;
        }

        let weight = edge_weight(&edge.edge_type, &edge.confidence);
        let confidence = confidence_from_weight(weight);
        let category = blast_edge_category(&edge.edge_type);
        add_candidate(
            &mut candidates,
            index,
            other,
            weight,
            confidence,
            category,
            format!("{}: {}", edge.edge_type, edge.reason),
        );
    }

    let mut impacted = candidates
        .into_values()
        .map(|candidate| candidate.into_impacted_file())
        .collect::<Vec<_>>();
    impacted.sort_by(|left, right| {
        blast_context_rank(&left.context)
            .cmp(&blast_context_rank(&right.context))
            .then_with(|| {
                right
                    .score
                    .partial_cmp(&left.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| {
                confidence_rank(&left.confidence).cmp(&confidence_rank(&right.confidence))
            })
            .then_with(|| left.path.cmp(&right.path))
    });
    impacted
}

fn build_inspection_order(
    focus_paths: &[String],
    impacted_files: &[BlastImpactedFile],
) -> Vec<String> {
    let mut order = focus_paths.to_vec();
    for file in impacted_files {
        if !order.iter().any(|existing| existing == &file.path) {
            order.push(file.path.clone());
        }
    }
    order
}

fn build_next_actions_for_file(
    _repo: &RepoInfo,
    query: &str,
    file_path: &str,
    affected_symbols: &[IndexedSymbol],
    index_status: &str,
    risk: &BlastRisk,
) -> Vec<String> {
    let mut actions = vec![format!("open {}", file_path)];
    push_unique_action(&mut actions, format!("agentgrep related {}", file_path));
    if let Some(best_symbol) = affected_symbols.first() {
        push_unique_action(
            &mut actions,
            format!("agentgrep symbol {}", best_symbol.name),
        );
    }
    push_unique_action(&mut actions, format!("agentgrep find \"{}\"", query));
    if index_status != "fresh" {
        push_unique_action(&mut actions, "agentgrep index --status".to_string());
    }
    if matches!(risk.level, BlastRiskLevel::Low) && index_status == "missing" {
        push_unique_action(&mut actions, "agentgrep index".to_string());
    }
    actions
}

fn build_next_actions_for_symbol(
    _repo: &RepoInfo,
    query: &str,
    symbol_report: &crate::types::SymbolReport,
    affected_symbols: &[IndexedSymbol],
    index_status: &str,
) -> Vec<String> {
    let mut actions = Vec::new();
    if let Some(first) = affected_symbols.first() {
        actions.push(format!("open {}", first.file_path));
    }
    push_unique_action(&mut actions, format!("agentgrep related {}", query));
    if symbol_report.matches.len() > 1 || symbol_report.match_mode != SymbolMatchMode::Exact {
        if let Some(first) = affected_symbols.first() {
            push_unique_action(&mut actions, format!("agentgrep symbol {}", first.name));
        }
    }
    push_unique_action(&mut actions, format!("agentgrep find \"{}\"", query));
    if index_status != "fresh" {
        push_unique_action(&mut actions, "agentgrep index --status".to_string());
    }
    if index_status == "missing" {
        push_unique_action(&mut actions, "agentgrep index".to_string());
    }
    actions
}

fn assess_risk(
    mode: BlastMode,
    impacted_files: &[BlastImpactedFile],
    references: &[IndexedSymbolReference],
    index_status: &str,
) -> BlastRisk {
    match mode {
        BlastMode::Symbol => assess_symbol_risk(impacted_files, references, index_status),
        BlastMode::File => assess_file_risk(impacted_files, references, index_status),
    }
}

fn assess_symbol_risk(
    impacted_files: &[BlastImpactedFile],
    references: &[IndexedSymbolReference],
    index_status: &str,
) -> BlastRisk {
    let totals = summarize_reference_totals(references);
    let production_files = totals.production_files;
    let test_fixture_files = totals.test_fixture_files;
    let same_area_files = impacted_files
        .iter()
        .filter(|file| matches!(file.context, BlastImpactContext::SameArea))
        .count();
    let mut reasons = Vec::new();

    if production_files >= 3 {
        reasons.push(format!(
            "{} production files have direct symbol users ({} production references)",
            production_files, totals.production_references
        ));
        if totals.test_fixture_references > 0 {
            reasons.push(format!(
                "{} test/fixture references across {} {}",
                totals.test_fixture_references,
                test_fixture_files,
                pluralize(test_fixture_files, "file", "files")
            ));
        }
        if index_status != "fresh" {
            reasons.push(format!("index status is {}", index_status));
        }
        return BlastRisk {
            level: BlastRiskLevel::High,
            reasons,
        };
    }

    if production_files > 0 {
        let verb = if production_files == 1 { "has" } else { "have" };
        reasons.push(format!(
            "{} production {} {} direct symbol users ({} production {})",
            production_files,
            pluralize(production_files, "file", "files"),
            verb,
            totals.production_references,
            pluralize(totals.production_references, "reference", "references")
        ));
        if totals.test_fixture_references > 0 {
            reasons.push(format!(
                "{} test/fixture references across {} {}",
                totals.test_fixture_references,
                test_fixture_files,
                pluralize(test_fixture_files, "file", "files")
            ));
        }
        if index_status != "fresh" {
            reasons.push(format!("index status is {}", index_status));
        }
        return BlastRisk {
            level: BlastRiskLevel::Medium,
            reasons,
        };
    }

    if test_fixture_files > 0 || totals.test_fixture_references > 0 {
        reasons.push(format!(
            "{} test/fixture references across {} {}",
            totals.test_fixture_references,
            test_fixture_files,
            pluralize(test_fixture_files, "file", "files")
        ));
    } else if same_area_files > 0 {
        reasons.push("only same_area evidence was found".to_string());
    } else if references.is_empty() {
        reasons.push("no direct symbol users were found".to_string());
    }

    if index_status != "fresh" {
        reasons.push(format!("index status is {}", index_status));
    }

    BlastRisk {
        level: BlastRiskLevel::Low,
        reasons,
    }
}

fn assess_file_risk(
    impacted_files: &[BlastImpactedFile],
    references: &[IndexedSymbolReference],
    index_status: &str,
) -> BlastRisk {
    let evidence = summarize_file_mode_evidence(impacted_files, references);
    let production_total_files = evidence.production_total_files();
    let production_symbol_files = evidence.production_symbol_files.len();
    let production_module_files = evidence.production_module_files.len();
    let test_fixture_files = evidence.test_fixture_files.len();
    let same_area_files = evidence.same_area_files.len();
    let mut reasons = Vec::new();
    let mut production_clauses = Vec::new();

    if evidence.production_symbol_references > 0 {
        production_clauses.push(format!(
            "{} production symbol {} across {} {}",
            evidence.production_symbol_references,
            pluralize(
                evidence.production_symbol_references,
                "reference",
                "references"
            ),
            production_symbol_files,
            pluralize(production_symbol_files, "file", "files")
        ));
    }
    if evidence.production_module_users > 0 {
        production_clauses.push(format!(
            "{} production file/module {} across {} {}",
            evidence.production_module_users,
            pluralize(evidence.production_module_users, "user", "users"),
            production_module_files,
            pluralize(production_module_files, "file", "files")
        ));
    }

    if production_total_files >= 3 {
        let verb = if production_total_files == 1 {
            "has"
        } else {
            "have"
        };
        reasons.push(format!(
            "{} production {} {} direct inbound impact ({})",
            production_total_files,
            pluralize(production_total_files, "file", "files"),
            verb,
            production_clauses.join("; ")
        ));
        if evidence.test_fixture_references > 0 {
            reasons.push(format!(
                "{} test/fixture references across {} {}",
                evidence.test_fixture_references,
                test_fixture_files,
                pluralize(test_fixture_files, "file", "files")
            ));
        }
        if index_status != "fresh" {
            reasons.push(format!("index status is {}", index_status));
        }
        return BlastRisk {
            level: BlastRiskLevel::High,
            reasons,
        };
    }

    if production_total_files > 0 {
        let verb = if production_total_files == 1 {
            "has"
        } else {
            "have"
        };
        reasons.push(format!(
            "{} production {} {} direct inbound impact ({})",
            production_total_files,
            pluralize(production_total_files, "file", "files"),
            verb,
            production_clauses.join("; ")
        ));
        if evidence.test_fixture_references > 0 {
            reasons.push(format!(
                "{} test/fixture references across {} {}",
                evidence.test_fixture_references,
                test_fixture_files,
                pluralize(test_fixture_files, "file", "files")
            ));
        }
        if index_status != "fresh" {
            reasons.push(format!("index status is {}", index_status));
        }
        return BlastRisk {
            level: BlastRiskLevel::Medium,
            reasons,
        };
    }

    if test_fixture_files > 0 || evidence.test_fixture_references > 0 {
        reasons.push(format!(
            "{} test/fixture references across {} {}",
            evidence.test_fixture_references,
            test_fixture_files,
            pluralize(test_fixture_files, "file", "files")
        ));
    } else if same_area_files > 0 {
        reasons.push("only same_area evidence was found".to_string());
    } else if references.is_empty() {
        reasons.push("no direct symbol or file users were found".to_string());
    }

    if index_status != "fresh" {
        reasons.push(format!("index status is {}", index_status));
    }

    BlastRisk {
        level: BlastRiskLevel::Low,
        reasons,
    }
}

fn render_symbol(symbol: &IndexedSymbol) {
    println!(
        "- {} [{} {}] {}:{}",
        symbol.name, symbol.kind, symbol.visibility, symbol.file_path, symbol.line_number
    );
    if let Some(signature) = &symbol.signature {
        println!("  signature: {}", signature);
    }
}

fn render_reference_section(
    mode: &BlastMode,
    references: &[IndexedSymbolReference],
    totals: ReferenceTotals,
) {
    if references.is_empty() {
        match mode {
            BlastMode::Symbol => println!("Direct symbol users: none"),
            BlastMode::File => println!("Symbol references: none"),
        }
        return;
    }

    let label = match mode {
        BlastMode::Symbol => "Direct symbol users",
        BlastMode::File => "Symbol references",
    };
    let production_summary = format!(
        "{} {} across {} {}",
        totals.production_references,
        pluralize(totals.production_references, "ref", "refs"),
        totals.production_files,
        pluralize(totals.production_files, "file", "files")
    );
    let test_fixture_summary = format!(
        "{} {} across {} {}",
        totals.test_fixture_references,
        pluralize(totals.test_fixture_references, "ref", "refs"),
        totals.test_fixture_files,
        pluralize(totals.test_fixture_files, "file", "files")
    );
    let unknown_summary = if totals.unknown_references > 0 {
        format!(
            "; unknown {} {} across {} {}",
            totals.unknown_references,
            pluralize(totals.unknown_references, "ref", "refs"),
            totals.unknown_files,
            pluralize(totals.unknown_files, "file", "files")
        )
    } else {
        String::new()
    };
    println!(
        "{} ({} total; production {}; test/fixture {}{}):",
        label, totals.total_references, production_summary, test_fixture_summary, unknown_summary
    );

    for reference in references.iter().take(REFERENCE_DISPLAY_LIMIT) {
        render_reference(reference);
    }
    if references.len() > REFERENCE_DISPLAY_LIMIT {
        println!(
            "- ... showing {} of {}",
            REFERENCE_DISPLAY_LIMIT,
            references.len()
        );
    }
}

fn render_impacted_files_section(files: &[BlastImpactedFile], max_score: f64) {
    if files.is_empty() {
        println!("Broader file/context impact: none");
        return;
    }

    let production = files
        .iter()
        .filter(|file| matches!(file.context, BlastImpactContext::Production))
        .count();
    let test_fixture = files
        .iter()
        .filter(|file| matches!(file.context, BlastImpactContext::TestFixture))
        .count();
    let same_area = files
        .iter()
        .filter(|file| matches!(file.context, BlastImpactContext::SameArea))
        .count();
    println!(
        "Broader file/context impact ({} total; production {}; test/fixture {}; same_area {}):",
        files.len(),
        production,
        test_fixture,
        same_area
    );
    for file in files.iter().take(IMPACT_FILE_DISPLAY_LIMIT) {
        render_impacted_file(file, max_score);
    }
    if files.len() > IMPACT_FILE_DISPLAY_LIMIT {
        println!(
            "- ... showing {} of {}",
            IMPACT_FILE_DISPLAY_LIMIT,
            files.len()
        );
    }
}

fn render_inspection_order(paths: &[String]) {
    if paths.is_empty() {
        return;
    }

    println!("Inspection order ({} total):", paths.len());
    for path in paths.iter().take(INSPECTION_ORDER_DISPLAY_LIMIT) {
        println!("- open {}", path);
    }
    if paths.len() > INSPECTION_ORDER_DISPLAY_LIMIT {
        println!(
            "- ... showing {} of {}",
            INSPECTION_ORDER_DISPLAY_LIMIT,
            paths.len()
        );
    }
}

fn render_impacted_file(file: &BlastImpactedFile, max_score: f64) {
    let normalized = normalized_score(file.score, max_score);
    println!(
        "- {} [{} / {}] impact {:.2} confidence {}",
        file.path, file.role, file.context, normalized, file.confidence
    );
    if !file.reasons.is_empty() {
        println!("  reasons: {}", file.reasons.join("; "));
    }
}

fn normalized_score(score: f64, max_score: f64) -> f64 {
    if max_score <= 0.0 {
        0.0
    } else {
        (score / max_score).min(1.0)
    }
}

fn pluralize<'a>(count: usize, singular: &'a str, plural: &'a str) -> &'a str {
    if count == 1 {
        singular
    } else {
        plural
    }
}

#[derive(Clone, Default)]
struct FileModeEvidence {
    production_symbol_files: BTreeSet<String>,
    production_module_files: BTreeSet<String>,
    test_fixture_files: BTreeSet<String>,
    same_area_files: BTreeSet<String>,
    unknown_files: BTreeSet<String>,
    production_symbol_references: usize,
    production_module_users: usize,
    test_fixture_references: usize,
    unknown_references: usize,
}

impl FileModeEvidence {
    fn production_total_files(&self) -> usize {
        self.production_symbol_files
            .union(&self.production_module_files)
            .count()
    }
}

fn render_reference(reference: &IndexedSymbolReference) {
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
        reference_context_priority(left.context)
            .cmp(&reference_context_priority(right.context))
            .then_with(|| {
                reference_confidence_priority(&left.confidence)
                    .cmp(&reference_confidence_priority(&right.confidence))
            })
            .then_with(|| left.from_file.cmp(&right.from_file))
            .then_with(|| left.line_number.cmp(&right.line_number))
            .then_with(|| left.symbol_name.cmp(&right.symbol_name))
            .then_with(|| left.reason.cmp(&right.reason))
    });
    grouped
}

fn summarize_reference_totals(references: &[IndexedSymbolReference]) -> ReferenceTotals {
    let mut production_files = BTreeSet::new();
    let mut test_fixture_files = BTreeSet::new();
    let mut unknown_files = BTreeSet::new();
    let mut totals = ReferenceTotals::default();

    for reference in references {
        let count = reference_total_count(reference);
        totals.total_references += count;
        match reference.context {
            ReferenceContext::Production => {
                totals.production_references += count;
                production_files.insert(reference.from_file.clone());
            }
            ReferenceContext::Test | ReferenceContext::Fixture => {
                totals.test_fixture_references += count;
                test_fixture_files.insert(reference.from_file.clone());
            }
            ReferenceContext::Unknown => {
                totals.unknown_references += count;
                unknown_files.insert(reference.from_file.clone());
            }
        }
    }

    totals.production_files = production_files.len();
    totals.test_fixture_files = test_fixture_files.len();
    totals.unknown_files = unknown_files.len();
    totals
}

fn summarize_file_mode_evidence(
    impacted_files: &[BlastImpactedFile],
    references: &[IndexedSymbolReference],
) -> FileModeEvidence {
    let mut evidence = FileModeEvidence::default();

    for reference in references {
        let count = reference_total_count(reference);
        match reference.context {
            ReferenceContext::Production => {
                evidence.production_symbol_references += count;
                evidence
                    .production_symbol_files
                    .insert(reference.from_file.clone());
            }
            ReferenceContext::Test | ReferenceContext::Fixture => {
                evidence.test_fixture_references += count;
                evidence
                    .test_fixture_files
                    .insert(reference.from_file.clone());
            }
            ReferenceContext::Unknown => {
                evidence.unknown_references += count;
                evidence.unknown_files.insert(reference.from_file.clone());
            }
        }
    }

    for file in impacted_files {
        match file.context {
            BlastImpactContext::Production => {
                if has_file_module_reason(&file.reasons) {
                    evidence.production_module_users += 1;
                    evidence.production_module_files.insert(file.path.clone());
                }
            }
            BlastImpactContext::TestFixture => {
                evidence.test_fixture_files.insert(file.path.clone());
            }
            BlastImpactContext::SameArea => {
                evidence.same_area_files.insert(file.path.clone());
            }
            BlastImpactContext::Unknown => {}
        }
    }

    evidence
}

fn reference_total_count(reference: &IndexedSymbolReference) -> usize {
    reference.additional_count + 1
}

fn has_file_module_reason(reasons: &[String]) -> bool {
    reasons.iter().any(|reason| {
        reason.starts_with("imports:")
            || reason.starts_with("references:")
            || reason.starts_with("declares_module:")
            || reason.starts_with("configures:")
    })
}

fn add_candidate(
    candidates: &mut BTreeMap<String, BlastImpactAccum>,
    index: &index::RepoIndex,
    path: &str,
    score: f64,
    confidence: Confidence,
    category: BlastImpactCategory,
    reason: String,
) {
    let role = file_role_for_path(index, path);
    let entry = candidates
        .entry(path.to_string())
        .or_insert_with(|| BlastImpactAccum::new(path.to_string(), role));
    entry.score += score;
    entry.confidence = max_confidence(entry.confidence.clone(), confidence);
    entry.reasons.insert(reason);
    entry.bump_category(category);
}

fn file_role_for_path(index: &index::RepoIndex, path: &str) -> String {
    index
        .files
        .iter()
        .find(|file| file.path == path)
        .map(|file| file.role.to_string())
        .unwrap_or_else(|| FileRole::Other.to_string())
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

fn is_blast_symbol_kind(kind: &crate::types::SymbolKind) -> bool {
    matches!(
        kind,
        crate::types::SymbolKind::Function
            | crate::types::SymbolKind::Struct
            | crate::types::SymbolKind::Enum
            | crate::types::SymbolKind::Trait
            | crate::types::SymbolKind::TypeAlias
            | crate::types::SymbolKind::Const
            | crate::types::SymbolKind::Static
            | crate::types::SymbolKind::Module
    )
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

fn edge_weight(edge_type: &str, confidence: &EdgeConfidence) -> f64 {
    let base = match edge_type {
        "imports" | "references" | "declares_module" | "configures" => 100.0,
        "likely_test_for" => 75.0,
        "same_area" => 20.0,
        _ => 40.0,
    };
    base + edge_confidence_bonus_enum(confidence)
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

fn max_confidence(left: Confidence, right: Confidence) -> Confidence {
    if confidence_rank(&right) < confidence_rank(&left) {
        right
    } else {
        left
    }
}

fn confidence_rank(confidence: &Confidence) -> usize {
    match confidence {
        Confidence::High => 0,
        Confidence::Medium => 1,
        Confidence::Low => 2,
    }
}

fn reference_context_priority(context: ReferenceContext) -> usize {
    match context {
        ReferenceContext::Production => 0,
        ReferenceContext::Fixture => 1,
        ReferenceContext::Test => 2,
        ReferenceContext::Unknown => 3,
    }
}

fn reference_confidence_priority(confidence: &EdgeConfidence) -> usize {
    match confidence {
        EdgeConfidence::Extracted => 0,
        EdgeConfidence::Inferred => 1,
        EdgeConfidence::Ambiguous => 2,
    }
}

fn other_edge_file<'a>(edge: &'a IndexedEdge, focus_set: &BTreeSet<String>) -> Option<&'a str> {
    if edge.edge_type == "same_area" {
        if focus_set.contains(&edge.from) && !focus_set.contains(&edge.to) {
            return Some(edge.to.as_str());
        }
        if focus_set.contains(&edge.to) && !focus_set.contains(&edge.from) {
            return Some(edge.from.as_str());
        }
        return None;
    }

    if focus_set.contains(&edge.to) && !focus_set.contains(&edge.from) {
        return Some(edge.from.as_str());
    }

    None
}

fn blast_edge_category(edge_type: &str) -> BlastImpactCategory {
    match edge_type {
        "imports" | "references" | "declares_module" | "configures" => {
            BlastImpactCategory::Production
        }
        "likely_test_for" => BlastImpactCategory::TestFixture,
        "same_area" => BlastImpactCategory::SameArea,
        _ => BlastImpactCategory::Unknown,
    }
}

fn blast_context_rank(context: &BlastImpactContext) -> usize {
    match context {
        BlastImpactContext::Production => 0,
        BlastImpactContext::TestFixture => 1,
        BlastImpactContext::SameArea => 2,
        BlastImpactContext::Unknown => 3,
    }
}

fn guess_mode(repo_root: &Path, input: &str) -> BlastMode {
    let resolved = map::resolve_requested_path(repo_root, input);
    if input.contains('/')
        || input.contains('\\')
        || input.contains('.')
        || Path::new(&resolved).exists()
    {
        BlastMode::File
    } else {
        BlastMode::Symbol
    }
}

fn push_unique_action(actions: &mut Vec<String>, action: String) {
    if !actions.iter().any(|existing| existing == &action) {
        actions.push(action);
    }
}

#[derive(Clone)]
struct BlastRisk {
    level: BlastRiskLevel,
    reasons: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BlastImpactCategory {
    Production,
    TestFixture,
    SameArea,
    Unknown,
}

struct BlastImpactAccum {
    path: String,
    role: String,
    score: f64,
    confidence: Confidence,
    reasons: BTreeSet<String>,
    category: BlastImpactCategory,
}

impl BlastImpactAccum {
    fn new(path: String, role: String) -> Self {
        Self {
            path,
            role,
            score: 0.0,
            confidence: Confidence::Low,
            reasons: BTreeSet::new(),
            category: BlastImpactCategory::Unknown,
        }
    }

    fn bump_category(&mut self, category: BlastImpactCategory) {
        self.category = match (self.category, category) {
            (BlastImpactCategory::Production, _) | (_, BlastImpactCategory::Production) => {
                BlastImpactCategory::Production
            }
            (BlastImpactCategory::TestFixture, _) | (_, BlastImpactCategory::TestFixture) => {
                BlastImpactCategory::TestFixture
            }
            (BlastImpactCategory::SameArea, _) | (_, BlastImpactCategory::SameArea) => {
                BlastImpactCategory::SameArea
            }
            _ => BlastImpactCategory::Unknown,
        };
    }

    fn into_impacted_file(self) -> BlastImpactedFile {
        BlastImpactedFile {
            path: self.path,
            role: self.role,
            score: self.score,
            confidence: self.confidence,
            context: match self.category {
                BlastImpactCategory::Production => BlastImpactContext::Production,
                BlastImpactCategory::TestFixture => BlastImpactContext::TestFixture,
                BlastImpactCategory::SameArea => BlastImpactContext::SameArea,
                BlastImpactCategory::Unknown => BlastImpactContext::Unknown,
            },
            reasons: self.reasons.into_iter().collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::{
        EdgeConfidence, FileRole, IndexState, IndexStats, IndexedEdge, ReferenceContext, RepoIndex,
    };
    use std::path::PathBuf;

    fn repo() -> RepoInfo {
        RepoInfo {
            root: PathBuf::from("C:/repo"),
            rev: Some("abc".to_string()),
            git_dir: None,
        }
    }

    fn file(path: &str) -> IndexedFile {
        IndexedFile {
            path: path.to_string(),
            role: FileRole::Source,
            size_bytes: Some(100),
            modified_unix: Some(1),
            content_hash: Some(format!("hash-{path}")),
            ..Default::default()
        }
    }

    fn symbol(
        name: &str,
        kind: crate::types::SymbolKind,
        file_path: &str,
        line_number: usize,
    ) -> IndexedSymbol {
        let signature = format!("pub {} {name}", kind);
        IndexedSymbol {
            name: name.to_string(),
            kind,
            file_path: file_path.to_string(),
            line_number,
            visibility: crate::types::Visibility::Public,
            signature: Some(signature),
            end_line: None,

            parent_class: None,        }
    }

    fn loaded_index() -> index::LoadedIndex {
        index::LoadedIndex {
            index_path: PathBuf::from("C:/repo/.agentgrep/index.json"),
            state: IndexState::Fresh,
            index: Some(RepoIndex {
                schema_version: crate::index::INDEX_SCHEMA_VERSION,
                repo_root: "C:/repo".to_string(),
                repo_rev: Some("abc".to_string()),
                indexed_at_unix: 1,
                files: vec![
                    file("src/search.rs"),
                    file("src/types.rs"),
                    file("src/main.rs"),
                    file("src/index.rs"),
                    file("src/symbol.rs"),
                    file("src/output.rs"),
                ],
                symbols: vec![
                    symbol(
                        "SearchResult",
                        crate::types::SymbolKind::Struct,
                        "src/search.rs",
                        11,
                    ),
                    symbol(
                        "FindReport",
                        crate::types::SymbolKind::Struct,
                        "src/types.rs",
                        5,
                    ),
                    symbol(
                        "SearchCoverage",
                        crate::types::SymbolKind::Struct,
                        "src/types.rs",
                        66,
                    ),
                    symbol(
                        "SearchCoverage",
                        crate::types::SymbolKind::Impl,
                        "src/types.rs",
                        77,
                    ),
                ],
                symbol_references: vec![
                    IndexedSymbolReference {
                        from_file: "src/main.rs".to_string(),
                        symbol_name: "SearchResult".to_string(),
                        target_file: Some("src/search.rs".to_string()),
                        target_line: Some(11),
                        line_number: 24,
                        confidence: EdgeConfidence::Extracted,
                        reason: "use statement reference".to_string(),
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
                        context: ReferenceContext::Fixture,
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
                    IndexedSymbolReference {
                        from_file: "src/index.rs".to_string(),
                        symbol_name: "SearchCoverage".to_string(),
                        target_file: Some("src/types.rs".to_string()),
                        target_line: Some(66),
                        line_number: 2500,
                        confidence: EdgeConfidence::Inferred,
                        reason: "qualified or token reference".to_string(),
                        context: ReferenceContext::Fixture,
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
                        edge_type: "declares_module".to_string(),
                        from: "src/main.rs".to_string(),
                        to: "src/search.rs".to_string(),
                        confidence: EdgeConfidence::Extracted,
                        reason: "declares module search".to_string(),
                    },
                    IndexedEdge {
                        edge_type: "same_area".to_string(),
                        from: "src/search.rs".to_string(),
                        to: "src/symbol.rs".to_string(),
                        confidence: EdgeConfidence::Extracted,
                        reason: "shared source area src".to_string(),
                    },
                ],
                stats: IndexStats {
                    file_count: 6,
                    role_counts: std::collections::BTreeMap::from([(FileRole::Source, 6)]),
                    symbol_count: 4,
                    symbol_kind_counts: std::collections::BTreeMap::new(),
                    symbol_reference_count: 6,
                    connection_count: 3,
                    ..Default::default()
                },
                        dep_imports: vec![],
            }),
        }
    }

    #[test]
    fn file_input_uses_file_mode() {
        let report = build_report_from_loaded(&repo(), &loaded_index(), "src/search.rs").unwrap();
        assert_eq!(report.mode, BlastMode::File);
        assert_eq!(report.query, "src/search.rs");
    }

    #[test]
    fn file_mode_treats_module_edges_as_production_impact() {
        let report = build_report_from_loaded(&repo(), &loaded_index(), "src/search.rs").unwrap();
        assert_eq!(report.mode, BlastMode::File);
        assert_eq!(report.risk_level, BlastRiskLevel::Medium);
        assert!(report
            .risk_reasons
            .first()
            .unwrap()
            .contains("production file/module"));
    }

    #[test]
    fn symbol_input_uses_symbol_mode() {
        let report = build_report_from_loaded(&repo(), &loaded_index(), "SearchResult").unwrap();
        assert_eq!(report.mode, BlastMode::Symbol);
        assert_eq!(report.affected_symbols.len(), 1);
    }

    #[test]
    fn production_references_rank_above_test_references() {
        let report = build_report_from_loaded(&repo(), &loaded_index(), "SearchResult").unwrap();
        assert!(!report.impacted_files.is_empty());
        assert_eq!(report.impacted_files[0].path, "src/main.rs");
    }

    #[test]
    fn symbol_mode_risk_is_based_on_direct_users_not_hub_edges() {
        let report = build_report_from_loaded(&repo(), &loaded_index(), "FindReport").unwrap();
        assert_eq!(report.risk_level, BlastRiskLevel::Medium);
        assert!(report
            .risk_reasons
            .first()
            .unwrap()
            .contains("1 production file"));
        assert!(!report
            .risk_reasons
            .first()
            .unwrap()
            .contains("9 production files"));
    }

    #[test]
    fn file_mode_high_risk_uses_unique_file_count() {
        let loaded = index::LoadedIndex {
            index_path: PathBuf::from("C:/repo/.agentgrep/index.json"),
            state: IndexState::Fresh,
            index: Some(RepoIndex {
                schema_version: crate::index::INDEX_SCHEMA_VERSION,
                repo_root: "C:/repo".to_string(),
                repo_rev: Some("abc".to_string()),
                indexed_at_unix: 1,
                files: vec![
                    file("src/types.rs"),
                    file("src/a.rs"),
                    file("src/b.rs"),
                    file("src/c.rs"),
                ],
                symbols: vec![symbol(
                    "FindReport",
                    crate::types::SymbolKind::Struct,
                    "src/types.rs",
                    5,
                )],
                symbol_references: vec![
                    IndexedSymbolReference {
                        from_file: "src/a.rs".to_string(),
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
                        from_file: "src/b.rs".to_string(),
                        symbol_name: "FindReport".to_string(),
                        target_file: Some("src/types.rs".to_string()),
                        target_line: Some(5),
                        line_number: 4,
                        confidence: EdgeConfidence::Extracted,
                        reason: "use statement reference".to_string(),
                        context: ReferenceContext::Production,
                        additional_count: 0,
                    },
                    IndexedSymbolReference {
                        from_file: "src/c.rs".to_string(),
                        symbol_name: "FindReport".to_string(),
                        target_file: Some("src/types.rs".to_string()),
                        target_line: Some(5),
                        line_number: 5,
                        confidence: EdgeConfidence::Extracted,
                        reason: "use statement reference".to_string(),
                        context: ReferenceContext::Production,
                        additional_count: 0,
                    },
                ],
                edges: vec![IndexedEdge {
                    edge_type: "same_area".to_string(),
                    from: "src/types.rs".to_string(),
                    to: "src/a.rs".to_string(),
                    confidence: EdgeConfidence::Extracted,
                    reason: "shared source area src".to_string(),
                }],
                stats: IndexStats {
                    file_count: 4,
                    role_counts: std::collections::BTreeMap::from([(FileRole::Source, 4)]),
                    symbol_count: 1,
                    symbol_kind_counts: std::collections::BTreeMap::new(),
                    symbol_reference_count: 3,
                    connection_count: 1,
                    ..Default::default()
                },
                        dep_imports: vec![],
            }),
        };
        let report = build_report_from_loaded(&repo(), &loaded, "src/types.rs").unwrap();
        assert_eq!(report.risk_level, BlastRiskLevel::High);
        assert!(report
            .risk_reasons
            .first()
            .unwrap()
            .contains("3 production files"));
    }

    #[test]
    fn same_area_alone_gives_low_risk() {
        let loaded = index::LoadedIndex {
            index_path: PathBuf::from("C:/repo/.agentgrep/index.json"),
            state: IndexState::Fresh,
            index: Some(RepoIndex {
                schema_version: crate::index::INDEX_SCHEMA_VERSION,
                repo_root: "C:/repo".to_string(),
                repo_rev: Some("abc".to_string()),
                indexed_at_unix: 1,
                files: vec![file("src/search.rs"), file("src/symbol.rs")],
                symbols: vec![symbol(
                    "SearchResult",
                    crate::types::SymbolKind::Struct,
                    "src/search.rs",
                    11,
                )],
                symbol_references: vec![],
                edges: vec![IndexedEdge {
                    edge_type: "same_area".to_string(),
                    from: "src/search.rs".to_string(),
                    to: "src/symbol.rs".to_string(),
                    confidence: EdgeConfidence::Extracted,
                    reason: "shared source area src".to_string(),
                }],
                stats: IndexStats {
                    file_count: 2,
                    role_counts: std::collections::BTreeMap::from([(FileRole::Source, 2)]),
                    symbol_count: 1,
                    symbol_kind_counts: std::collections::BTreeMap::new(),
                    symbol_reference_count: 0,
                    connection_count: 1,
                    ..Default::default()
                },
                        dep_imports: vec![],
            }),
        };
        let report = build_report_from_loaded(&repo(), &loaded, "src/search.rs").unwrap();
        assert_eq!(report.risk_level, BlastRiskLevel::Low);
        assert!(report
            .risk_reasons
            .iter()
            .any(|reason| reason.contains("same_area")));
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
    fn json_includes_risk_level_and_impacted_files() {
        let report = build_report_from_loaded(&repo(), &loaded_index(), "SearchResult").unwrap();
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["risk_level"], "medium");
        assert!(json["impacted_files"].is_array());
    }
}

