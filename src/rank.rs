use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};

use crate::index::{self, EdgeConfidence, IndexedEdge, IndexedSymbolReference, ReferenceContext};
use crate::text::{normalize_phrase, shorten_snippet, squash_identifier, tokenize_terms};
use crate::types::{
    Confidence, Evidence, FileCandidate, IndexedSymbol, LineRange, SearchMatch, Snippet,
};

pub const CANDIDATE_LIMIT: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindRoleFilter {
    Source,
    Doc,
    Config,
    Test,
    Other,
    Any,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindMatchFilter {
    Any,
    All,
}

#[derive(Debug, Clone)]
pub struct FindFilters {
    include: GlobMatcherSet,
    exclude: GlobMatcherSet,
    role: FindRoleFilter,
    match_mode: FindMatchFilter,
}

impl FindFilters {
    pub fn try_new(
        include: Vec<String>,
        exclude: Vec<String>,
        role: FindRoleFilter,
        match_mode: FindMatchFilter,
    ) -> Result<Self> {
        Ok(Self {
            include: build_globset(include, "include")?,
            exclude: build_globset(exclude, "exclude")?,
            role,
            match_mode,
        })
    }

    fn allows(&self, path: &str, role: &str) -> bool {
        let normalized_path = path.replace('\\', "/");
        if !self.include.is_empty() && !self.include.is_match(&normalized_path) {
            return false;
        }
        if !self.exclude.is_empty() && self.exclude.is_match(&normalized_path) {
            return false;
        }
        role_matches(self.role, role)
    }
}

impl Default for FindFilters {
    fn default() -> Self {
        Self {
            include: GlobMatcherSet::empty(),
            exclude: GlobMatcherSet::empty(),
            role: FindRoleFilter::Any,
            match_mode: FindMatchFilter::Any,
        }
    }
}

pub fn rank_with_index(
    query: &str,
    matches: Vec<SearchMatch>,
    index: Option<&index::RepoIndex>,
    index_status: &str,
    filters: &FindFilters,
) -> Vec<FileCandidate> {
    let profile = QueryProfile::new(query);
    let mut grouped: BTreeMap<String, Vec<SearchMatch>> = BTreeMap::new();

    for item in matches {
        grouped.entry(item.path.clone()).or_default().push(item);
    }

    let candidates = grouped
        .into_iter()
        .map(|(path, mut matches)| {
            matches.sort_by_key(|item| item.line_number);
            build_candidate(path, matches, &profile, index, index_status)
        })
        .filter(|ranked| filters.allows(&ranked.candidate.path, &ranked.candidate.role))
        .filter(|ranked| match filters.match_mode {
            FindMatchFilter::Any => true,
            FindMatchFilter::All => ranked.matched_terms >= profile.terms.len(),
        })
        .collect::<Vec<_>>();

    let mut finalized = candidates
        .into_iter()
        .map(finalize_ranked_candidate)
        .collect::<Vec<_>>();

    finalized.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.path.cmp(&right.path))
    });

    finalized.into_iter().take(CANDIDATE_LIMIT).collect()
}

pub fn next_actions(_query: &str, candidates: &[FileCandidate], repo_root: &str) -> Vec<String> {
    let mut actions = Vec::new();

    for candidate in candidates.iter().take(3) {
        let action = format!("open {}", candidate.path);
        if !actions.contains(&action) {
            actions.push(action);
        }
    }

    if actions.is_empty() {
        actions.push(format!("open {}", repo_root));
    }

    actions
}

struct QueryProfile {
    normalized_phrase: String,
    squashed_phrase: String,
    terms: Vec<String>,
    identifier_like: bool,
}

impl QueryProfile {
    fn new(query: &str) -> Self {
        let raw = query.trim().to_string();
        let normalized_phrase = normalize_phrase(&raw);
        let squashed_phrase = squash_identifier(&normalized_phrase);
        let mut terms = tokenize_terms(&raw);
        terms.sort();
        terms.dedup();
        let identifier_like = is_identifier_like(&raw, &terms);

        Self {
            normalized_phrase,
            squashed_phrase,
            terms,
            identifier_like,
        }
    }
}

fn build_candidate(
    path: String,
    matches: Vec<SearchMatch>,
    profile: &QueryProfile,
    index: Option<&index::RepoIndex>,
    index_status: &str,
) -> RankedCandidate {
    let normalized_path = path.replace('\\', "/");
    let lower_path = normalized_path.to_lowercase();
    let path_tokens = tokenize_terms(&normalized_path);
    let file_name = normalized_path
        .rsplit('/')
        .next()
        .unwrap_or(normalized_path.as_str())
        .to_string();
    let file_name_tokens = tokenize_terms(&file_name);

    let role = classify_role(&lower_path, &file_name_tokens);
    let clusters = cluster_matches(&matches);
    let file_has_test_signals = file_has_test_signals(&matches);
    let best_exact_cluster = select_exact_phrase_cluster(&clusters, profile);
    let best_near_cluster = if best_exact_cluster.is_none() {
        select_near_phrase_cluster(&clusters, profile)
    } else {
        None
    };

    let mut evidence = Vec::new();
    let mut score = 0.0;
    let mut matched_terms = BTreeSet::new();
    let mut path_shape_tier = 0usize;

    for token_match in collect_token_matches(profile, &path_tokens, &file_name_tokens, &matches) {
        score += token_match.score;
        if let Some(evidence_item) = token_match.evidence {
            evidence.push(evidence_item);
        }
        matched_terms.insert(token_match.term);
    }

    if let Some((boost, tier, evidence_item)) =
        filename_shape_boost(profile, &path_tokens, &file_name_tokens)
    {
        score += boost;
        evidence.push(evidence_item);
        path_shape_tier = path_shape_tier.max(tier);
    }

    if let Some(cluster) = best_exact_cluster {
        let fixture_like = cluster.is_fixture_like() || file_has_test_signals;
        let phrase_boost = exact_phrase_boost(&role, profile, fixture_like);
        score += phrase_boost;
        evidence.push(Evidence {
            evidence_type: "exact_phrase_match".to_string(),
            detail: format!(
                "matched exact phrase in lines {}-{}",
                cluster.start_line, cluster.end_line
            ),
        });
        if fixture_like {
            score -= 0.45;
            evidence.push(Evidence {
                evidence_type: "fixture_like_match".to_string(),
                detail: "exact phrase appears in assertion or fixture-like text".to_string(),
            });
        }
    } else if let Some(cluster) = best_near_cluster {
        score += near_phrase_boost(&role, profile);
        evidence.push(Evidence {
            evidence_type: "near_phrase_match".to_string(),
            detail: format!(
                "{} query terms clustered in lines {}-{}",
                cluster.term_hits(profile),
                cluster.start_line,
                cluster.end_line
            ),
        });
    }

    apply_role_weight(&role, &mut score, &mut evidence);

    let mut index_tier = path_shape_tier;
    if let Some(index) = index {
        index_tier = index_tier.max(apply_index_evidence(
            &normalized_path,
            &role,
            profile,
            index,
            index_status,
            &mut score,
            &mut evidence,
        ));
    }

    let snippets = build_snippets(&clusters, profile);
    if !snippets.is_empty() {
        score += 0.01 * snippets.len().min(3) as f64;
    }

    if !matches.is_empty() {
        let line_ranges = compress_line_ranges(&matches);
        let lines = format_line_ranges(&line_ranges);
        score += 0.03 + (clusters.len().min(4) as f64 * 0.01);
        evidence.push(Evidence {
            evidence_type: "rg_match".to_string(),
            detail: format!("matched on lines {lines}"),
        });
    }

    if !profile.terms.is_empty() && !matched_terms.is_empty() {
        let matched_term_count = matched_terms.len() as f64;
        let missing_terms = profile.terms.len().saturating_sub(matched_terms.len()) as f64;
        score += matched_term_count * 0.14;
        if missing_terms > 0.0 {
            score -= missing_terms * 0.07;
        }
        evidence.push(Evidence {
            evidence_type: "query_term_coverage".to_string(),
            detail: format!(
                "matched {} of {} query terms",
                matched_terms.len(),
                profile.terms.len()
            ),
        });
    }

    if evidence
        .iter()
        .any(|item| item.evidence_type == "fixture_like_match")
    {
        score = score.min(0.30);
    }

    let score = round_score(score.clamp(0.0, 1.0));
    let confidence = confidence_for(
        &role,
        profile,
        score,
        &evidence,
        matched_terms.len(),
        &snippets,
    );

    RankedCandidate {
        candidate: FileCandidate {
            path,
            kind: "file".to_string(),
            role: role.to_string(),
            score,
            confidence,
            line_ranges: compress_line_ranges(&matches),
            snippets,
            evidence,
        },
        tier: index_tier,
        matched_terms: matched_terms.len(),
    }
}

struct TokenMatch {
    term: String,
    score: f64,
    evidence: Option<Evidence>,
}

fn collect_token_matches(
    profile: &QueryProfile,
    path_tokens: &[String],
    file_name_tokens: &[String],
    matches: &[SearchMatch],
) -> Vec<TokenMatch> {
    let mut results = Vec::new();

    for term in &profile.terms {
        let filename_hit = file_name_tokens
            .iter()
            .any(|token| token_matches_term(term, token));
        let path_hit = path_tokens
            .iter()
            .any(|token| token_matches_term(term, token));
        let snippet_hit = matches
            .iter()
            .any(|item| normalize_phrase(&item.snippet).contains(term));

        if filename_hit {
            results.push(TokenMatch {
                term: term.clone(),
                score: if profile.identifier_like { 0.20 } else { 0.16 },
                evidence: Some(Evidence {
                    evidence_type: "filename_token_match".to_string(),
                    detail: format!("filename token matches '{term}'"),
                }),
            });
        } else if path_hit {
            results.push(TokenMatch {
                term: term.clone(),
                score: if profile.identifier_like { 0.12 } else { 0.08 },
                evidence: Some(Evidence {
                    evidence_type: "path_token_match".to_string(),
                    detail: format!("path token matches '{term}'"),
                }),
            });
        }

        if snippet_hit {
            results.push(TokenMatch {
                term: term.clone(),
                score: if profile.identifier_like { 0.09 } else { 0.05 },
                evidence: Some(Evidence {
                    evidence_type: "snippet_term_match".to_string(),
                    detail: format!("matched '{term}' in snippet"),
                }),
            });
        }
    }

    results
}

fn filename_shape_boost(
    profile: &QueryProfile,
    path_tokens: &[String],
    file_name_tokens: &[String],
) -> Option<(f64, usize, Evidence)> {
    if profile.terms.is_empty() {
        return None;
    }

    let exact_file_match = profile
        .terms
        .iter()
        .all(|term| file_name_tokens.iter().any(|token| token == term));
    let fuzzy_file_match = profile.terms.iter().all(|term| {
        file_name_tokens
            .iter()
            .any(|token| token_matches_term(term, token))
    });

    let exact_path_match = profile
        .terms
        .iter()
        .all(|term| path_tokens.iter().any(|token| token == term));
    let fuzzy_path_match = profile.terms.iter().all(|term| {
        path_tokens
            .iter()
            .any(|token| token_matches_term(term, token))
    });

    if exact_file_match {
        let boost = if profile.identifier_like { 0.42 } else { 0.62 };
        return Some((
            boost,
            5,
            Evidence {
                evidence_type: "filename_shape_match".to_string(),
                detail: format!("filename stem matches '{}'", profile.normalized_phrase),
            },
        ));
    }

    if fuzzy_file_match {
        let boost = if profile.identifier_like { 0.34 } else { 0.50 };
        return Some((
            boost,
            5,
            Evidence {
                evidence_type: "filename_shape_match".to_string(),
                detail: format!(
                    "filename stem closely matches '{}'",
                    profile.normalized_phrase
                ),
            },
        ));
    }

    if exact_path_match {
        let boost = if profile.identifier_like { 0.22 } else { 0.32 };
        return Some((
            boost,
            4,
            Evidence {
                evidence_type: "path_shape_match".to_string(),
                detail: format!("path tokens match '{}'", profile.normalized_phrase),
            },
        ));
    }

    if fuzzy_path_match {
        let boost = if profile.identifier_like { 0.16 } else { 0.26 };
        return Some((
            boost,
            4,
            Evidence {
                evidence_type: "path_shape_match".to_string(),
                detail: format!("path tokens closely match '{}'", profile.normalized_phrase),
            },
        ));
    }

    None
}

fn token_matches_term(term: &str, token: &str) -> bool {
    term == token || singularize_token(term) == token || singularize_token(token) == term
}

fn singularize_token(token: &str) -> String {
    if token.len() <= 3 {
        return token.to_string();
    }

    if let Some(stripped) = token.strip_suffix("ies") {
        return format!("{stripped}y");
    }

    for suffix in ["ses", "xes", "zes", "ches", "shes"] {
        if let Some(stripped) = token.strip_suffix(suffix) {
            return stripped.to_string();
        }
    }

    if token.ends_with('s') && !token.ends_with("ss") {
        return token[..token.len() - 1].to_string();
    }

    token.to_string()
}

fn exact_phrase_boost(role: &FileRole, profile: &QueryProfile, fixture_like: bool) -> f64 {
    let mut boost = match role {
        FileRole::Source => 0.34,
        FileRole::Doc => 0.24,
        FileRole::Test => 0.18,
        FileRole::Config => 0.14,
        FileRole::Lockfile => 0.08,
        FileRole::Generated => 0.10,
        FileRole::Other => 0.20,
    };

    if profile.identifier_like {
        boost += 0.04;
    }

    if fixture_like {
        boost -= 0.12;
    }

    boost
}

fn near_phrase_boost(role: &FileRole, profile: &QueryProfile) -> f64 {
    let mut boost = match role {
        FileRole::Source => 0.14,
        FileRole::Doc => 0.08,
        FileRole::Test => 0.08,
        FileRole::Config => 0.06,
        FileRole::Lockfile => 0.02,
        FileRole::Generated => 0.06,
        FileRole::Other => 0.08,
    };

    if profile.identifier_like {
        boost += 0.02;
    }

    boost
}

fn apply_role_weight(role: &FileRole, score: &mut f64, evidence: &mut Vec<Evidence>) {
    match role {
        FileRole::Source => {
            *score += 0.06;
            evidence.push(Evidence {
                evidence_type: "source_role".to_string(),
                detail: "path suggests source-like file role".to_string(),
            });
        }
        FileRole::Test => {
            *score += 0.03;
            evidence.push(Evidence {
                evidence_type: "test_role".to_string(),
                detail: "path suggests test file role".to_string(),
            });
        }
        FileRole::Doc => {
            *score += 0.02;
            evidence.push(Evidence {
                evidence_type: "doc_role".to_string(),
                detail: "path suggests documentation file role".to_string(),
            });
        }
        FileRole::Config => {
            *score += 0.04;
            evidence.push(Evidence {
                evidence_type: "config_role".to_string(),
                detail: "path suggests configuration file role".to_string(),
            });
        }
        FileRole::Lockfile => {
            *score -= 0.10;
            evidence.push(Evidence {
                evidence_type: "lockfile_role".to_string(),
                detail: "path suggests lockfile or dependency snapshot".to_string(),
            });
        }
        FileRole::Generated => {
            *score -= 0.05;
            evidence.push(Evidence {
                evidence_type: "generated_role".to_string(),
                detail: "path suggests generated or build output".to_string(),
            });
        }
        FileRole::Other => {}
    }
}

fn apply_index_evidence(
    path: &str,
    role: &FileRole,
    profile: &QueryProfile,
    index: &index::RepoIndex,
    index_status: &str,
    score: &mut f64,
    evidence: &mut Vec<Evidence>,
) -> usize {
    let scale = index_boost_scale(index_status);
    if scale <= 0.0 {
        return 0;
    }

    let mut boosts = Vec::new();
    let mut seen = BTreeSet::new();

    for symbol in index
        .symbols
        .iter()
        .filter(|symbol| symbol.file_path == path)
    {
        if let Some(signal) = symbol_definition_signal(symbol, profile, role, scale) {
            let key = format!("definition:{}:{}", symbol.name, symbol.line_number);
            if seen.insert(key) {
                boosts.push(signal);
            }
        }
    }

    for reference in index.symbol_references.iter().filter(|reference| {
        reference.from_file == path && reference_matches_profile(reference, profile)
    }) {
        if let Some(signal) = symbol_reference_signal(reference, scale) {
            let key = format!(
                "reference:{}:{}:{}",
                reference.symbol_name, reference.line_number, reference.reason
            );
            if seen.insert(key) {
                boosts.push(signal);
            }
        }
    }

    for edge in index
        .edges
        .iter()
        .filter(|edge| edge.from == path || edge.to == path)
    {
        if let Some(signal) = edge_signal(edge, path, scale) {
            let key = format!("edge:{}:{}:{}", edge.edge_type, edge.from, edge.to);
            if seen.insert(key) {
                boosts.push(signal);
            }
        }
    }

    boosts.sort_by(|left, right| {
        right
            .tier
            .cmp(&left.tier)
            .then_with(|| {
                right
                    .score
                    .partial_cmp(&left.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| {
                index_confidence_priority(left.confidence)
                    .cmp(&index_confidence_priority(right.confidence))
            })
            .then_with(|| {
                left.evidence
                    .evidence_type
                    .cmp(&right.evidence.evidence_type)
            })
    });
    let index_tier = boosts.iter().map(|signal| signal.tier).max().unwrap_or(0);
    boosts.truncate(3);

    for signal in boosts.into_iter().rev() {
        *score += signal.score;
        evidence.insert(0, signal.evidence);
    }

    index_tier
}

fn index_boost_scale(index_status: &str) -> f64 {
    match index_status {
        "fresh" => 1.0,
        "stale" => 0.85,
        "unverifiable" => 0.8,
        "missing" => 0.0,
        _ => 0.8,
    }
}

fn symbol_definition_signal(
    symbol: &IndexedSymbol,
    profile: &QueryProfile,
    role: &FileRole,
    scale: f64,
) -> Option<IndexSignal> {
    let strength = symbol_match_strength(&symbol.name, profile)?;
    let (base, tier, confidence) = match strength {
        SymbolMatchStrength::Exact => {
            let base = match role {
                FileRole::Source => {
                    if profile.identifier_like {
                        0.68
                    } else {
                        0.44
                    }
                }
                FileRole::Doc => 0.32,
                FileRole::Test => 0.44,
                FileRole::Config => 0.36,
                FileRole::Lockfile => 0.16,
                FileRole::Generated => 0.06,
                FileRole::Other => 0.40,
            };
            let tier = if matches!(role, FileRole::Source) {
                5
            } else {
                4
            };
            (base, tier, Confidence::High)
        }
        SymbolMatchStrength::Strong => (0.28, 3, Confidence::Medium),
        SymbolMatchStrength::Loose => (0.14, 2, Confidence::Low),
    };
    let mut score = base * scale;

    if score <= 0.0 {
        return None;
    }

    if matches!(strength, SymbolMatchStrength::Exact) && matches!(role, FileRole::Source) {
        score += 0.10 * scale;
    }

    Some(IndexSignal {
        score,
        confidence,
        tier,
        evidence: Evidence {
            evidence_type: "indexed_symbol_definition".to_string(),
            detail: format!("defines symbol {}", symbol.name),
        },
    })
}

fn symbol_reference_signal(reference: &IndexedSymbolReference, scale: f64) -> Option<IndexSignal> {
    let (base, tier) = match reference.context {
        ReferenceContext::Production => (0.16, 2),
        ReferenceContext::Fixture => (0.04, 1),
        ReferenceContext::Test => (0.03, 1),
        ReferenceContext::Unknown => (0.01, 0),
    };
    let confidence_bonus = match reference.confidence {
        EdgeConfidence::Extracted => 0.03,
        EdgeConfidence::Inferred => 0.0,
        EdgeConfidence::Ambiguous => -0.01,
    };
    let confidence = match reference.confidence {
        EdgeConfidence::Extracted => Confidence::High,
        EdgeConfidence::Inferred => Confidence::Medium,
        EdgeConfidence::Ambiguous => Confidence::Low,
    };
    let score = if base + confidence_bonus > 0.0 {
        (base + confidence_bonus) * scale
    } else {
        0.0
    };
    if score <= 0.0 {
        return None;
    }

    Some(IndexSignal {
        score,
        confidence,
        tier,
        evidence: Evidence {
            evidence_type: "indexed_symbol_reference".to_string(),
            detail: format!("references symbol {}", reference.symbol_name),
        },
    })
}

fn edge_signal(edge: &IndexedEdge, path: &str, scale: f64) -> Option<IndexSignal> {
    let (base, tier) = match edge.edge_type.as_str() {
        "imports" | "references" | "declares_module" => (0.05, 1),
        "same_area" => (0.005, 0),
        _ => (0.01, 0),
    };
    let directional_bonus = if edge.to == path && edge.edge_type != "same_area" {
        0.01
    } else {
        0.0
    };
    let score = (base + directional_bonus) * scale;
    if score <= 0.0 {
        return None;
    }

    Some(IndexSignal {
        score,
        confidence: if edge.edge_type == "same_area" {
            Confidence::Low
        } else {
            Confidence::Medium
        },
        tier,
        evidence: Evidence {
            evidence_type: "indexed_edge".to_string(),
            detail: format!("indexed edge {} {}", edge.edge_type, edge.reason),
        },
    })
}

fn reference_matches_profile(reference: &IndexedSymbolReference, profile: &QueryProfile) -> bool {
    let name = reference.symbol_name.to_lowercase();
    if !profile.squashed_phrase.is_empty() && squash_identifier(&name) == profile.squashed_phrase {
        return true;
    }

    if !profile.normalized_phrase.is_empty() && name == profile.normalized_phrase {
        return true;
    }

    let name_terms = tokenize_terms(&reference.symbol_name);
    profile
        .terms
        .iter()
        .all(|term| name_terms.iter().any(|name_term| name_term == term))
}

#[derive(Debug, Clone)]
struct GlobMatcherSet {
    path: GlobSet,
    basename: GlobSet,
}

impl GlobMatcherSet {
    fn empty() -> Self {
        Self {
            path: GlobSetBuilder::new()
                .build()
                .expect("empty globset should be valid"),
            basename: GlobSetBuilder::new()
                .build()
                .expect("empty globset should be valid"),
        }
    }

    fn is_empty(&self) -> bool {
        self.path.is_empty() && self.basename.is_empty()
    }

    fn is_match(&self, path: &str) -> bool {
        let basename = path.rsplit('/').next().unwrap_or(path);
        self.path.is_match(path) || self.basename.is_match(basename)
    }
}

fn build_globset(patterns: Vec<String>, label: &str) -> Result<GlobMatcherSet> {
    let mut path_builder = GlobSetBuilder::new();
    let mut basename_builder = GlobSetBuilder::new();
    for pattern in patterns {
        let glob = Glob::new(&pattern)
            .with_context(|| format!("invalid {label} glob pattern: {pattern}"))?;
        path_builder.add(glob);

        if !pattern.contains('/') && !pattern.contains('\\') {
            let basename_glob = Glob::new(&pattern)
                .with_context(|| format!("invalid {label} glob pattern: {pattern}"))?;
            basename_builder.add(basename_glob);
        }
    }
    Ok(GlobMatcherSet {
        path: path_builder.build()?,
        basename: basename_builder.build()?,
    })
}

fn role_matches(filter: FindRoleFilter, role: &str) -> bool {
    match filter {
        FindRoleFilter::Any => true,
        FindRoleFilter::Source => role == "source",
        FindRoleFilter::Doc => role == "doc",
        FindRoleFilter::Config => role == "config",
        FindRoleFilter::Test => role == "test",
        FindRoleFilter::Other => matches!(role, "other" | "lockfile" | "generated"),
    }
}

fn is_generated_site_html(path: &str) -> bool {
    let lower = path.to_lowercase();
    (lower.contains("/site/") || lower.starts_with("site/")) && lower.ends_with("index.html")
}

#[derive(Clone, Copy)]
enum SymbolMatchStrength {
    Exact,
    Strong,
    Loose,
}

fn symbol_match_strength(name: &str, profile: &QueryProfile) -> Option<SymbolMatchStrength> {
    let normalized = normalize_phrase(name);
    let squashed = squash_identifier(name);

    if !profile.squashed_phrase.is_empty() && squashed == profile.squashed_phrase {
        return Some(SymbolMatchStrength::Exact);
    }

    if !profile.normalized_phrase.is_empty() && normalized == profile.normalized_phrase {
        return Some(SymbolMatchStrength::Exact);
    }

    if !profile.squashed_phrase.is_empty() && squashed.contains(&profile.squashed_phrase) {
        return Some(SymbolMatchStrength::Strong);
    }

    if profile
        .terms
        .iter()
        .all(|term| squash_identifier(name).contains(term))
    {
        return Some(SymbolMatchStrength::Strong);
    }

    if profile.terms.iter().any(|term| {
        tokenize_terms(name)
            .iter()
            .any(|name_term| name_term == term)
    }) {
        return Some(SymbolMatchStrength::Loose);
    }

    None
}

struct IndexSignal {
    score: f64,
    confidence: Confidence,
    tier: usize,
    evidence: Evidence,
}

struct RankedCandidate {
    candidate: FileCandidate,
    tier: usize,
    matched_terms: usize,
}

fn finalize_ranked_candidate(mut ranked: RankedCandidate) -> FileCandidate {
    let tier = ranked.tier;
    let has_index_definition = ranked
        .candidate
        .evidence
        .iter()
        .any(|item| item.evidence_type == "indexed_symbol_definition");
    let has_index_reference = ranked
        .candidate
        .evidence
        .iter()
        .any(|item| item.evidence_type == "indexed_symbol_reference");
    let has_index_edge = ranked
        .candidate
        .evidence
        .iter()
        .any(|item| item.evidence_type == "indexed_edge");

    if tier > 0 {
        let tier_score = tier as f64 * 0.15;
        ranked.candidate.score = round_score((ranked.candidate.score + tier_score).max(0.0));
        ranked.candidate.confidence = if has_index_definition {
            Confidence::High
        } else if tier >= 2 || has_index_reference {
            Confidence::Medium
        } else if has_index_edge {
            Confidence::Low
        } else {
            ranked.candidate.confidence
        };
    }

    ranked.candidate
}

fn index_confidence_priority(confidence: Confidence) -> usize {
    match confidence {
        Confidence::High => 0,
        Confidence::Medium => 1,
        Confidence::Low => 2,
    }
}

fn confidence_for(
    role: &FileRole,
    profile: &QueryProfile,
    score: f64,
    evidence: &[Evidence],
    matched_terms: usize,
    snippets: &[Snippet],
) -> Confidence {
    let has_exact_phrase = evidence
        .iter()
        .any(|item| item.evidence_type == "exact_phrase_match");
    let has_near_phrase = evidence
        .iter()
        .any(|item| item.evidence_type == "near_phrase_match");
    let has_source_role = evidence
        .iter()
        .any(|item| item.evidence_type == "source_role");
    let has_fixture_like_match = evidence
        .iter()
        .any(|item| item.evidence_type == "fixture_like_match");
    let has_strong_snippet = snippets
        .first()
        .map(|snippet| snippet.text.len() <= 80)
        .unwrap_or(false)
        && has_exact_phrase
        && !has_fixture_like_match;
    let role_is_source = matches!(role, FileRole::Source);

    if has_fixture_like_match {
        return Confidence::Low;
    }

    if has_strong_snippet || (profile.identifier_like && role_is_source && has_exact_phrase) {
        return Confidence::High;
    }

    if has_exact_phrase || has_near_phrase || (role_is_source && matched_terms >= 2) {
        return Confidence::Medium;
    }

    if has_source_role && score >= 0.60 {
        return Confidence::Medium;
    }

    Confidence::Low
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FileRole {
    Source,
    Test,
    Doc,
    Config,
    Lockfile,
    Generated,
    Other,
}

impl std::fmt::Display for FileRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            FileRole::Source => "source",
            FileRole::Test => "test",
            FileRole::Doc => "doc",
            FileRole::Config => "config",
            FileRole::Lockfile => "lockfile",
            FileRole::Generated => "generated",
            FileRole::Other => "other",
        };
        write!(f, "{value}")
    }
}

#[derive(Clone, Debug)]
struct MatchCluster {
    start_line: usize,
    end_line: usize,
    matches: Vec<SearchMatch>,
}

impl MatchCluster {
    fn joined_text(&self) -> String {
        self.matches
            .iter()
            .map(|item| item.snippet.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn span(&self) -> usize {
        self.end_line.saturating_sub(self.start_line) + 1
    }

    fn term_hits(&self, profile: &QueryProfile) -> usize {
        let joined = normalize_phrase(&self.joined_text());
        profile
            .terms
            .iter()
            .filter(|term| joined.contains(term.as_str()))
            .count()
    }

    fn has_exact_phrase(&self, profile: &QueryProfile) -> bool {
        if profile.normalized_phrase.is_empty() {
            return false;
        }

        let joined = normalize_phrase(&self.joined_text());
        joined.contains(&profile.normalized_phrase)
            || (!profile.squashed_phrase.is_empty()
                && squash_identifier(&joined).contains(&profile.squashed_phrase))
    }

    fn is_fixture_like(&self) -> bool {
        self.matches.iter().any(|item| {
            let text = item.snippet.to_lowercase();
            text.contains("assert")
                || text.contains("expect(")
                || text.contains("fixture")
                || text.contains("example")
                || text.contains("sample")
                || text.contains("mock")
                || text.contains("test")
        })
    }
}

fn cluster_matches(matches: &[SearchMatch]) -> Vec<MatchCluster> {
    let mut clusters = Vec::new();
    let mut current: Option<MatchCluster> = None;

    for item in matches.iter().cloned() {
        match current.as_mut() {
            Some(cluster) if item.line_number <= cluster.end_line + 2 => {
                cluster.end_line = item.line_number;
                cluster.matches.push(item);
            }
            Some(_) => {
                clusters.push(current.take().unwrap());
                current = Some(MatchCluster {
                    start_line: item.line_number,
                    end_line: item.line_number,
                    matches: vec![item],
                });
            }
            None => {
                current = Some(MatchCluster {
                    start_line: item.line_number,
                    end_line: item.line_number,
                    matches: vec![item],
                });
            }
        }
    }

    if let Some(cluster) = current {
        clusters.push(cluster);
    }

    clusters
}

fn select_exact_phrase_cluster<'a>(
    clusters: &'a [MatchCluster],
    profile: &QueryProfile,
) -> Option<&'a MatchCluster> {
    clusters
        .iter()
        .filter(|cluster| cluster.has_exact_phrase(profile))
        .max_by(|left, right| {
            cluster_priority(left, profile)
                .partial_cmp(&cluster_priority(right, profile))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
}

fn select_near_phrase_cluster<'a>(
    clusters: &'a [MatchCluster],
    profile: &QueryProfile,
) -> Option<&'a MatchCluster> {
    clusters
        .iter()
        .filter(|cluster| cluster.term_hits(profile) >= 2)
        .max_by(|left, right| {
            cluster_priority(left, profile)
                .partial_cmp(&cluster_priority(right, profile))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
}

fn build_snippets(clusters: &[MatchCluster], profile: &QueryProfile) -> Vec<Snippet> {
    let mut ranked_clusters = clusters.iter().collect::<Vec<_>>();
    ranked_clusters.sort_by(|left, right| {
        cluster_priority(right, profile)
            .partial_cmp(&cluster_priority(left, profile))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.start_line.cmp(&right.start_line))
    });

    ranked_clusters
        .into_iter()
        .take(3)
        .filter_map(|cluster| {
            let best_match = cluster.matches.iter().max_by(|left, right| {
                snippet_score(left, profile)
                    .partial_cmp(&snippet_score(right, profile))
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| left.line_number.cmp(&right.line_number))
            })?;

            Some(Snippet {
                line_number: best_match.line_number,
                text: shorten_snippet(&best_match.snippet, 88),
            })
        })
        .collect()
}

fn cluster_priority(cluster: &MatchCluster, profile: &QueryProfile) -> f64 {
    let exact = if cluster.has_exact_phrase(profile) {
        1.0
    } else {
        0.0
    };
    let term_hits = cluster.term_hits(profile) as f64;
    let density = if cluster.span() == 0 {
        0.0
    } else {
        term_hits / cluster.span() as f64
    };
    let fixture_penalty = if cluster.is_fixture_like() { 0.25 } else { 0.0 };

    exact * 4.0 + term_hits * 1.2 + density * 1.5 - fixture_penalty
}

fn snippet_score(snippet: &SearchMatch, profile: &QueryProfile) -> f64 {
    let text = normalize_phrase(&snippet.snippet);
    let exact =
        if !profile.normalized_phrase.is_empty() && text.contains(&profile.normalized_phrase) {
            1.0
        } else {
            0.0
        };
    let squashed_exact = if !profile.squashed_phrase.is_empty()
        && squash_identifier(&text).contains(&profile.squashed_phrase)
    {
        1.0
    } else {
        0.0
    };
    let term_hits = profile
        .terms
        .iter()
        .filter(|term| text.contains(term.as_str()))
        .count() as f64;
    let fixture_penalty = if is_fixture_like_text(&text) {
        0.15
    } else {
        0.0
    };

    exact * 4.0 + squashed_exact * 2.5 + term_hits * 0.8 - fixture_penalty
}

fn is_fixture_like_text(text: &str) -> bool {
    text.contains("assert")
        || text.contains("expect(")
        || text.contains("fixture")
        || text.contains("example")
        || text.contains("sample")
        || text.contains("mock")
        || text.contains("test")
}

fn file_has_test_signals(matches: &[SearchMatch]) -> bool {
    matches.iter().any(|item| {
        let text = item.snippet.to_lowercase();
        text.contains("#[test]")
            || text.contains("mod tests")
            || text.contains("assert_eq!")
            || text.contains("assert!")
            || text.contains("match_item(")
            || text.contains("test ")
    })
}

fn classify_role(path: &str, file_name_tokens: &[String]) -> FileRole {
    if is_generated_path(path) {
        return FileRole::Generated;
    }

    if is_lockfile(path) {
        return FileRole::Lockfile;
    }

    if is_test_path(path) {
        return FileRole::Test;
    }

    if is_doc_path(path) {
        return FileRole::Doc;
    }

    if is_config_path(path) {
        return FileRole::Config;
    }

    if is_source_path(path, file_name_tokens) {
        return FileRole::Source;
    }

    FileRole::Other
}

fn is_identifier_like(raw: &str, terms: &[String]) -> bool {
    raw.contains('_')
        || raw.contains("--")
        || raw.contains("::")
        || raw.contains('/')
        || raw.chars().any(|ch| ch.is_ascii_uppercase())
        || terms
            .iter()
            .any(|term| term.len() >= 6 && raw.contains(term))
}

fn is_lockfile(path: &str) -> bool {
    let lower = path.to_lowercase();
    lower.ends_with("cargo.lock")
        || lower.ends_with("package-lock.json")
        || lower.ends_with("pnpm-lock.yaml")
        || lower.ends_with("yarn.lock")
        || lower.ends_with("poetry.lock")
        || lower.ends_with("composer.lock")
}

fn is_test_path(path: &str) -> bool {
    path.contains("/test")
        || path.contains("/tests")
        || path.contains("__tests__")
        || path.contains("test_")
        || path.ends_with("_test.rs")
        || path.ends_with(".spec.")
        || path.ends_with(".test.")
}

fn is_config_path(path: &str) -> bool {
    path.contains("config")
        || path.contains("settings")
        || path.ends_with(".toml")
        || path.ends_with(".yaml")
        || path.ends_with(".yml")
        || path.ends_with(".json")
}

fn is_doc_path(path: &str) -> bool {
    path.ends_with(".md")
        || path.ends_with(".rst")
        || path.contains("/docs/")
        || path.contains("/doc/")
        || path.ends_with("readme")
        || path.ends_with("readme.md")
}

fn is_generated_path(path: &str) -> bool {
    path.contains("/target/")
        || path.contains("/dist/")
        || path.contains("/build/")
        || path.contains("/vendor/")
        || path.contains("generated")
        || is_generated_site_html(path)
}

fn is_source_path(path: &str, file_name_tokens: &[String]) -> bool {
    let lower = path.to_lowercase();
    let source_extensions = [
        ".rs", ".ts", ".tsx", ".js", ".jsx", ".py", ".go", ".java", ".c", ".cc", ".cpp", ".h",
        ".hpp", ".cs", ".rb", ".php", ".swift", ".kt", ".kts", ".scala", ".sh", ".sql", ".html",
        ".css", ".scss",
    ];

    source_extensions.iter().any(|ext| lower.ends_with(ext))
        || lower.contains("/src/")
        || lower.contains("/app/")
        || lower.contains("/lib/")
        || lower.contains("/services/")
        || lower.contains("/routes/")
        || lower.contains("/pages/")
        || lower.contains("/components/")
        || lower.contains("/server/")
        || file_name_tokens
            .iter()
            .any(|token| token == "src" || token == "lib" || token == "app")
}

fn compress_line_ranges(matches: &[SearchMatch]) -> Vec<LineRange> {
    let mut lines = matches
        .iter()
        .map(|item| item.line_number)
        .collect::<Vec<_>>();
    lines.sort_unstable();
    lines.dedup();

    let mut ranges = Vec::new();
    let mut current_start = None;
    let mut current_end = None;

    for line in lines {
        match (current_start, current_end) {
            (None, None) => {
                current_start = Some(line);
                current_end = Some(line);
            }
            (Some(start), Some(end)) if line == end + 1 => {
                current_start = Some(start);
                current_end = Some(line);
            }
            (Some(start), Some(end)) => {
                ranges.push(LineRange { start, end });
                current_start = Some(line);
                current_end = Some(line);
            }
            _ => {}
        }
    }

    if let (Some(start), Some(end)) = (current_start, current_end) {
        ranges.push(LineRange { start, end });
    }

    ranges
}

fn format_line_ranges(ranges: &[LineRange]) -> String {
    ranges
        .iter()
        .map(|range| {
            if range.start == range.end {
                range.start.to_string()
            } else {
                format!("{}-{}", range.start, range.end)
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn round_score(score: f64) -> f64 {
    (score * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::{
        EdgeConfidence, FileRole as IndexFileRole, IndexStats, IndexedEdge, IndexedFile,
        IndexedSymbolReference, ReferenceContext, RepoIndex,
    };
    use std::collections::BTreeMap;

    fn match_item(path: &str, line_number: usize, snippet: &str) -> SearchMatch {
        SearchMatch {
            path: path.to_string(),
            line_number,
            snippet: snippet.to_string(),
        }
    }

    fn indexed_symbol(name: &str, file_path: &str, line_number: usize) -> IndexedSymbol {
        IndexedSymbol {
            name: name.to_string(),
            kind: crate::types::SymbolKind::Struct,
            file_path: file_path.to_string(),
            line_number,
            visibility: crate::types::Visibility::Public,
            signature: Some(format!("pub struct {name} {{")),
        }
    }

    fn indexed_file(path: &str, role: IndexFileRole) -> IndexedFile {
        IndexedFile {
            path: path.to_string(),
            role,
            size_bytes: None,
            modified_unix: None,
            content_hash: None,
        }
    }

    fn repo_index(
        symbols: Vec<IndexedSymbol>,
        references: Vec<IndexedSymbolReference>,
        edges: Vec<IndexedEdge>,
        files: Vec<IndexedFile>,
    ) -> RepoIndex {
        RepoIndex {
            schema_version: crate::index::INDEX_SCHEMA_VERSION,
            repo_root: "C:/repo".to_string(),
            repo_rev: Some("abc".to_string()),
            indexed_at_unix: 1,
            files,
            symbols,
            symbol_references: references,
            edges,
            stats: IndexStats {
                file_count: 2,
                role_counts: BTreeMap::from([(IndexFileRole::Source, 2)]),
                symbol_count: 0,
                symbol_kind_counts: BTreeMap::new(),
                symbol_reference_count: 0,
                connection_count: 0,
            },
        }
    }

    #[test]
    fn token_aware_path_matching_does_not_match_cargo_lock_for_rg() {
        let matches = vec![
            match_item("Cargo.lock", 1, "rg"),
            match_item("src/lib.rs", 4, "rg"),
        ];

        let candidates = rank_with_index(
            "rg",
            matches,
            None,
            "not_applicable",
            &FindFilters::default(),
        );
        assert_eq!(candidates.first().unwrap().path, "src/lib.rs");
        assert_ne!(candidates.first().unwrap().path, "Cargo.lock");
    }

    #[test]
    fn exact_phrase_query_ranks_source_over_docs() {
        let matches = vec![
            match_item(
                "docs/PROJECT.md",
                20,
                "Agentgrep is not a repo chatbot and it is not a dashboard.",
            ),
            match_item("docs/ROADMAP.md", 32, "Do not add model-based features."),
            match_item(
                "src/search.rs",
                22,
                "rg was not found on PATH. Install ripgrep and try again.",
            ),
        ];

        let candidates = rank_with_index(
            "rg was not found",
            matches,
            None,
            "not_applicable",
            &FindFilters::default(),
        );
        assert_eq!(candidates.first().unwrap().path, "src/search.rs");
        assert!(candidates
            .first()
            .unwrap()
            .evidence
            .iter()
            .any(|item| item.evidence_type == "exact_phrase_match"));
    }

    #[test]
    fn exact_phrase_snippet_is_selected_over_weaker_matches() {
        let matches = vec![
            match_item("src/search.rs", 5, "let query = build_patterns(query);"),
            match_item(
                "src/search.rs",
                22,
                "rg was not found on PATH. Install ripgrep and try again.",
            ),
            match_item("src/search.rs", 40, "return Err(anyhow!(\"rg failed\"));"),
        ];

        let candidates = rank_with_index(
            "rg was not found",
            matches,
            None,
            "not_applicable",
            &FindFilters::default(),
        );
        let candidate = candidates.first().unwrap();
        assert_eq!(candidate.path, "src/search.rs");
        assert!(!candidate.snippets.is_empty());
        assert!(candidate.snippets[0]
            .text
            .contains("rg was not found on PATH"));
    }

    #[test]
    fn lockfile_is_penalized_without_dependency_context() {
        let matches = vec![
            match_item("Cargo.lock", 1, "query"),
            match_item("src/main.rs", 8, "query"),
        ];

        let candidates = rank_with_index(
            "query",
            matches,
            None,
            "not_applicable",
            &FindFilters::default(),
        );
        assert_eq!(candidates.first().unwrap().path, "src/main.rs");
        let cargo = candidates
            .iter()
            .find(|candidate| candidate.path == "Cargo.lock")
            .unwrap();
        let source = candidates
            .iter()
            .find(|candidate| candidate.path == "src/main.rs")
            .unwrap();
        assert!(cargo.score < source.score);
    }

    #[test]
    fn source_wins_over_docs_for_identifier_like_queries() {
        let matches = vec![
            match_item("docs/README.md", 10, "line_ranges are described here"),
            match_item("src/types.rs", 20, "pub struct LineRanges;"),
        ];

        let candidates = rank_with_index(
            "line_ranges",
            matches,
            None,
            "not_applicable",
            &FindFilters::default(),
        );
        assert_eq!(candidates.first().unwrap().path, "src/types.rs");
        assert_eq!(candidates.first().unwrap().role, "source");
    }

    #[test]
    fn builds_short_snippets() {
        let matches = vec![
            match_item(
                "src/main.rs",
                10,
                "this is a very long snippet that should be truncated for display",
            ),
            match_item("src/main.rs", 12, "another line"),
            match_item("src/main.rs", 30, "third line"),
        ];

        let candidates = rank_with_index(
            "snippet",
            matches,
            None,
            "not_applicable",
            &FindFilters::default(),
        );
        let snippets = &candidates.first().unwrap().snippets;
        assert!(!snippets.is_empty());
        assert!(snippets.iter().all(|snippet| snippet.text.len() <= 88));
        assert!(snippets.len() <= 3);
    }

    #[test]
    fn confidence_reflects_signal_quality() {
        let strong = vec![match_item(
            "src/search.rs",
            4,
            "let search_report = SearchReport::new();",
        )];
        let weak = vec![
            match_item("docs/README.md", 4, "search"),
            match_item("docs/README.md", 200, "report"),
        ];

        let strong_candidate = rank_with_index(
            "SearchReport",
            strong,
            None,
            "not_applicable",
            &FindFilters::default(),
        )
        .remove(0);
        let weak_candidate = rank_with_index(
            "search report",
            weak,
            None,
            "not_applicable",
            &FindFilters::default(),
        )
        .remove(0);

        assert_eq!(strong_candidate.confidence, Confidence::High);
        assert_eq!(weak_candidate.confidence, Confidence::Low);
    }

    #[test]
    fn exact_symbol_definition_ranks_definition_file_first_with_index() {
        let matches = vec![
            match_item(
                "src/search.rs",
                11,
                "pub struct SearchResult {",
            ),
            match_item(
                "src/blast.rs",
                1492,
                "let report = build_report_from_loaded(&repo(), &loaded_index(), \"SearchResult\").unwrap();",
            ),
            match_item(
                "src/related.rs",
                1241,
                "let report = build_report_from_loaded(&repo(), &loaded_index(), \"SearchResult\").unwrap();",
            ),
        ];
        let index = repo_index(
            vec![indexed_symbol("SearchResult", "src/search.rs", 11)],
            vec![
                IndexedSymbolReference {
                    from_file: "src/blast.rs".to_string(),
                    symbol_name: "SearchResult".to_string(),
                    target_file: Some("src/search.rs".to_string()),
                    target_line: Some(11),
                    line_number: 1492,
                    confidence: EdgeConfidence::Inferred,
                    reason: "qualified or token reference".to_string(),
                    context: ReferenceContext::Test,
                    additional_count: 0,
                },
                IndexedSymbolReference {
                    from_file: "src/related.rs".to_string(),
                    symbol_name: "SearchResult".to_string(),
                    target_file: Some("src/search.rs".to_string()),
                    target_line: Some(11),
                    line_number: 1241,
                    confidence: EdgeConfidence::Inferred,
                    reason: "qualified or token reference".to_string(),
                    context: ReferenceContext::Fixture,
                    additional_count: 0,
                },
            ],
            vec![
                IndexedEdge {
                    edge_type: "same_area".to_string(),
                    from: "src/blast.rs".to_string(),
                    to: "src/search.rs".to_string(),
                    confidence: EdgeConfidence::Extracted,
                    reason: "shared source area src".to_string(),
                },
                IndexedEdge {
                    edge_type: "references".to_string(),
                    from: "src/related.rs".to_string(),
                    to: "src/search.rs".to_string(),
                    confidence: EdgeConfidence::Inferred,
                    reason: "references crate::search".to_string(),
                },
            ],
            vec![
                indexed_file("src/search.rs", IndexFileRole::Source),
                indexed_file("src/blast.rs", IndexFileRole::Source),
                indexed_file("src/related.rs", IndexFileRole::Source),
            ],
        );

        let candidates = rank_with_index(
            "SearchResult",
            matches,
            Some(&index),
            "fresh",
            &FindFilters::default(),
        );
        assert_eq!(candidates.first().unwrap().path, "src/search.rs");
        assert!(candidates
            .first()
            .unwrap()
            .evidence
            .iter()
            .any(|item| item.evidence_type == "indexed_symbol_definition"));
    }

    #[test]
    fn topic_queries_rank_matching_filenames_ahead_of_broad_symbol_hubs() {
        let matches = vec![
            match_item("app/models.py", 18, "class MeetingSession(Model):"),
            match_item("app/meeting_session.py", 22, "def start_session():"),
            match_item("app/routers/meeting_sessions.py", 14, "def build_router():"),
        ];
        let index = repo_index(
            vec![
                indexed_symbol("MeetingSession", "app/models.py", 18),
                indexed_symbol("start_session", "app/meeting_session.py", 22),
            ],
            vec![],
            vec![],
            vec![
                indexed_file("app/models.py", IndexFileRole::Source),
                indexed_file("app/meeting_session.py", IndexFileRole::Source),
                indexed_file("app/routers/meeting_sessions.py", IndexFileRole::Source),
            ],
        );

        let candidates = rank_with_index(
            "meeting session",
            matches,
            Some(&index),
            "fresh",
            &FindFilters::default(),
        );
        assert_eq!(candidates.first().unwrap().path, "app/meeting_session.py");
        assert_ne!(candidates.first().unwrap().path, "app/models.py");
    }

    #[test]
    fn exact_symbol_reference_stays_below_definition_with_index() {
        let matches = vec![
            match_item(
                "src/types.rs",
                66,
                "pub struct SearchCoverage {",
            ),
            match_item(
                "src/index.rs",
                2500,
                "let coverage = SearchCoverage::new();",
            ),
            match_item(
                "src/symbol.rs",
                625,
                "let report = build_report_from_loaded(&repo(), &loaded, \"SearchCoverage\").unwrap();",
            ),
        ];
        let index = repo_index(
            vec![indexed_symbol("SearchCoverage", "src/types.rs", 66)],
            vec![
                IndexedSymbolReference {
                    from_file: "src/index.rs".to_string(),
                    symbol_name: "SearchCoverage".to_string(),
                    target_file: Some("src/types.rs".to_string()),
                    target_line: Some(66),
                    line_number: 2500,
                    confidence: EdgeConfidence::Extracted,
                    reason: "use statement reference".to_string(),
                    context: ReferenceContext::Production,
                    additional_count: 0,
                },
                IndexedSymbolReference {
                    from_file: "src/symbol.rs".to_string(),
                    symbol_name: "SearchCoverage".to_string(),
                    target_file: Some("src/types.rs".to_string()),
                    target_line: Some(66),
                    line_number: 625,
                    confidence: EdgeConfidence::Inferred,
                    reason: "qualified or token reference".to_string(),
                    context: ReferenceContext::Fixture,
                    additional_count: 0,
                },
            ],
            vec![
                IndexedEdge {
                    edge_type: "imports".to_string(),
                    from: "src/index.rs".to_string(),
                    to: "src/types.rs".to_string(),
                    confidence: EdgeConfidence::Extracted,
                    reason: "imports crate::types".to_string(),
                },
                IndexedEdge {
                    edge_type: "same_area".to_string(),
                    from: "src/symbol.rs".to_string(),
                    to: "src/types.rs".to_string(),
                    confidence: EdgeConfidence::Extracted,
                    reason: "shared source area src".to_string(),
                },
            ],
            vec![
                indexed_file("src/types.rs", IndexFileRole::Source),
                indexed_file("src/index.rs", IndexFileRole::Source),
                indexed_file("src/symbol.rs", IndexFileRole::Source),
            ],
        );

        let candidates = rank_with_index(
            "SearchCoverage",
            matches,
            Some(&index),
            "fresh",
            &FindFilters::default(),
        );
        assert_eq!(candidates.first().unwrap().path, "src/types.rs");
        assert!(candidates
            .first()
            .unwrap()
            .evidence
            .iter()
            .any(|item| item.evidence_type == "indexed_symbol_definition"));
    }

    #[test]
    fn stronger_source_matches_outrank_doc_noise() {
        let matches = vec![
            match_item("docs/PROJECT.md", 20, "rg was not found on PATH"),
            match_item(
                "src/search.rs",
                22,
                "rg was not found on PATH. Install ripgrep and try again.",
            ),
            match_item("src/blast.rs", 550, "rg was not found on PATH"),
        ];
        let index = repo_index(
            vec![indexed_symbol("SearchResult", "src/search.rs", 11)],
            vec![],
            vec![
                IndexedEdge {
                    edge_type: "same_area".to_string(),
                    from: "src/blast.rs".to_string(),
                    to: "src/search.rs".to_string(),
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
            vec![
                indexed_file("src/search.rs", IndexFileRole::Source),
                indexed_file("src/blast.rs", IndexFileRole::Source),
                indexed_file("docs/PROJECT.md", IndexFileRole::Doc),
            ],
        );

        let candidates = rank_with_index(
            "rg was not found",
            matches,
            Some(&index),
            "fresh",
            &FindFilters::default(),
        );
        assert_eq!(candidates.first().unwrap().path, "src/search.rs");
    }

    #[test]
    fn filters_include_exclude_and_role_limit_displayed_candidates() {
        let matches = vec![
            match_item("site/generated/index.html", 1, "download"),
            match_item("src/main.rs", 2, "download"),
            match_item("docs/README.md", 3, "download"),
        ];
        let include_css = FindFilters::try_new(
            vec!["**/*.html".to_string()],
            vec!["site/**".to_string()],
            FindRoleFilter::Any,
            FindMatchFilter::Any,
        )
        .unwrap();
        let source_only =
            FindFilters::try_new(vec![], vec![], FindRoleFilter::Source, FindMatchFilter::Any)
                .unwrap();

        let excluded = rank_with_index(
            "download",
            vec![
                match_item("site/generated/index.html", 1, "download"),
                match_item("src/main.rs", 2, "download"),
                match_item("docs/README.md", 3, "download"),
            ],
            None,
            "not_applicable",
            &include_css,
        );
        assert!(excluded.is_empty());

        let filtered = rank_with_index("download", matches, None, "not_applicable", &source_only);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].path, "src/main.rs");
    }

    #[test]
    fn explicit_include_allows_css_results() {
        let matches = vec![
            match_item("src/main.ts", 1, "background"),
            match_item("src/styles/site.css", 4, "background: #fff;"),
        ];
        let css_only = FindFilters::try_new(
            vec!["*.css".to_string()],
            vec![],
            FindRoleFilter::Any,
            FindMatchFilter::Any,
        )
        .unwrap();

        let candidates = rank_with_index("background", matches, None, "not_applicable", &css_only);

        assert_eq!(candidates.first().unwrap().path, "src/styles/site.css");
    }

    #[test]
    fn double_star_include_also_matches_nested_css() {
        let matches = vec![
            match_item("src/main.ts", 1, "background"),
            match_item("src/popup/popup.css", 4, "background: #fff;"),
        ];
        let css_only = FindFilters::try_new(
            vec!["**/*.css".to_string()],
            vec![],
            FindRoleFilter::Any,
            FindMatchFilter::Any,
        )
        .unwrap();

        let candidates = rank_with_index("background", matches, None, "not_applicable", &css_only);

        assert_eq!(candidates.first().unwrap().path, "src/popup/popup.css");
    }

    #[test]
    fn path_specific_include_matches_nested_css() {
        let matches = vec![
            match_item("src/main.ts", 1, "background"),
            match_item("src/options/options.css", 4, "background: #fff;"),
        ];
        let css_only = FindFilters::try_new(
            vec!["src/**/*.css".to_string()],
            vec![],
            FindRoleFilter::Any,
            FindMatchFilter::Any,
        )
        .unwrap();

        let candidates = rank_with_index("background", matches, None, "not_applicable", &css_only);

        assert_eq!(candidates.first().unwrap().path, "src/options/options.css");
    }

    #[test]
    fn exclude_globs_remove_nested_css() {
        let matches = vec![
            match_item("src/main.ts", 1, "background"),
            match_item("src/popup/popup.css", 4, "background: #fff;"),
        ];
        let filters = FindFilters::try_new(
            vec![],
            vec!["*.css".to_string()],
            FindRoleFilter::Any,
            FindMatchFilter::Any,
        )
        .unwrap();

        let candidates = rank_with_index("background", matches, None, "not_applicable", &filters);

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].path, "src/main.ts");
    }

    #[test]
    fn multi_term_coverage_beats_one_term_incidental_match() {
        let matches = vec![
            match_item("src/styles/site.css", 12, "background: #111;"),
            match_item(
                "src/mixed/file.ts",
                8,
                "background script registers handler",
            ),
            match_item("manifest.json", 4, "\"background\": {"),
        ];
        let index = repo_index(
            vec![],
            vec![],
            vec![IndexedEdge {
                edge_type: "references".to_string(),
                from: "manifest.json".to_string(),
                to: "src/mixed/file.ts".to_string(),
                confidence: EdgeConfidence::Extracted,
                reason: "manifest background.service_worker references src/mixed/file.ts"
                    .to_string(),
            }],
            vec![
                indexed_file("manifest.json", IndexFileRole::Config),
                indexed_file("src/mixed/file.ts", IndexFileRole::Source),
                indexed_file("src/styles/site.css", IndexFileRole::Source),
            ],
        );

        let candidates = rank_with_index(
            "background script",
            matches,
            Some(&index),
            "fresh",
            &FindFilters::default(),
        );

        assert_eq!(candidates.first().unwrap().path, "src/mixed/file.ts");
        assert!(
            candidates
                .iter()
                .find(|candidate| candidate.path == "src/styles/site.css")
                .map(|candidate| candidate.score)
                .unwrap()
                < candidates.first().unwrap().score
        );
    }

    #[test]
    fn match_all_filters_out_partial_matches() {
        let matches = vec![
            match_item("src/styles/site.css", 12, "background: #111;"),
            match_item(
                "src/mixed/file.ts",
                8,
                "background script registers handler",
            ),
            match_item("manifest.json", 4, "\"background\": {"),
        ];
        let filters =
            FindFilters::try_new(vec![], vec![], FindRoleFilter::Any, FindMatchFilter::All)
                .unwrap();

        let candidates = rank_with_index(
            "background script",
            matches,
            None,
            "not_applicable",
            &filters,
        );

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].path, "src/mixed/file.ts");
    }

    #[test]
    fn css_is_not_globally_penalized() {
        let matches = vec![
            match_item("src/main.ts", 1, "background"),
            match_item("src/styles/site.css", 4, "background color: #fff;"),
        ];

        let candidates = rank_with_index(
            "background color",
            matches,
            None,
            "not_applicable",
            &FindFilters::default(),
        );

        assert_eq!(candidates.first().unwrap().path, "src/styles/site.css");
    }

    #[test]
    fn generated_files_are_lower_evidence_not_hidden() {
        let matches = vec![
            match_item("site/docs/index.html", 18, "route handler"),
            match_item("src/routes/handler.ts", 12, "route handler"),
        ];
        let index = repo_index(
            vec![],
            vec![],
            vec![],
            vec![
                indexed_file("site/docs/index.html", IndexFileRole::Generated),
                indexed_file("src/routes/handler.ts", IndexFileRole::Source),
            ],
        );

        let candidates = rank_with_index(
            "route handler",
            matches,
            Some(&index),
            "fresh",
            &FindFilters::default(),
        );

        assert_eq!(candidates.first().unwrap().path, "src/routes/handler.ts");
    }
}
