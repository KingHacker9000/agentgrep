use std::collections::{BTreeMap, BTreeSet, HashMap};

use anyhow::{Context, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};

use crate::index::{self, EdgeConfidence, IndexedEdge, IndexedSymbolReference, ReferenceContext};
use crate::text::{
    normalize_phrase, shorten_snippet, squash_identifier, tokenize_lexical, tokenize_terms,
};
use crate::types::{
    Confidence, DetailLevel, Evidence, FileCandidate, IndexedSymbol, LineRange, SearchMatch,
    Snippet, SymbolSummary,
};

pub const CANDIDATE_LIMIT: usize = 8;
/// Hard cap on the number of candidates returned regardless of score.
pub const CANDIDATE_ENUM_LIMIT: usize = 50;

/// Per-signal-family score budget.  Each bucket is capped independently before
/// summing, so no single family can push the total above 1.0.  Budgets:
///   filename_shape 0.40 — filename/path token match boost (separate so it can
///                          outweigh phrase+symbol when the file is *named* after the query)
///   lexical        0.20 — token matches, BM25, snippets, rg hit, term coverage
///   phrase         0.18 — exact/near phrase (can go negative for fixture penalty)
///   sym_def        0.25 — indexed_symbol_definition
///   reference      0.08 — indexed_symbol_reference + indexed_edge
///   role           0.08 — source/doc/test/config bonus; lockfile/generated penalty
/// Max sum: 0.40+0.20+0.18+0.25+0.08+0.08 = 1.19 → clamped to 1.0
struct ScoreBudget {
    filename_shape: f64,
    lexical: f64,
    phrase: f64,
    symbol_def: f64,
    reference: f64,
    role: f64,
}

impl ScoreBudget {
    fn new() -> Self {
        Self {
            filename_shape: 0.0,
            lexical: 0.0,
            phrase: 0.0,
            symbol_def: 0.0,
            reference: 0.0,
            role: 0.0,
        }
    }

    fn total(&self) -> f64 {
        let shape = self.filename_shape.clamp(0.0, 0.40);
        let lex = self.lexical.clamp(0.0, 0.20);
        let ph = self.phrase.clamp(-0.30, 0.18);
        // Raised from 0.25 → 0.35 so that 2 exact symbol definitions (0.22×2=0.44)
        // score clearly higher than 1 (0.22) and cap isn't hit by a single match.
        let sym = self.symbol_def.clamp(0.0, 0.35);
        let rf = self.reference.clamp(0.0, 0.08);
        let role = self.role.clamp(-0.15, 0.08);
        (shape + lex + ph + sym + rf + role).clamp(0.0, 1.0)
    }
}

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
    /// When true, hard-exclude doc, lockfile, and generated files regardless of score.
    pub exclude_docs: bool,
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
            exclude_docs: false,
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
        if self.exclude_docs
            && matches!(role, "doc" | "lockfile" | "generated")
        {
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
            exclude_docs: false,
        }
    }
}

/// Shared lexical ranking entry point for all deterministic eval modes.
///
/// - Mode B (no index): called with `index = None`; only lexical signals score.
/// - Mode C (indexed):  called with the loaded `RepoIndex`; symbol definitions,
///   references, graph edges, and BM25 lex scores are all applied on top of the
///   same lexical candidate set.
/// - Mode D (semantic): identical to C here; `semantic::expand_candidates` is
///   called by the caller *after* this function returns and can re-sort the list
///   (score += similarity×0.3 for existing candidates; semantic-only candidates
///   added at similarity×0.8) unless the query is identifier-like, in which case
///   only annotation evidence is added and deterministic order is preserved.
pub fn rank_with_index(
    query: &str,
    matches: Vec<SearchMatch>,
    index: Option<&index::RepoIndex>,
    index_status: &str,
    filters: &FindFilters,
) -> Vec<FileCandidate> {
    let profile = QueryProfile::new(query);

    // Precompute document frequency (DF) per lex token across indexed files.
    // DF = number of files where the term appears in the top-300 term_frequencies.
    let lex_df: HashMap<String, usize> = index
        .filter(|_| !profile.lex_tokens.is_empty())
        .map(|idx| {
            profile
                .lex_tokens
                .iter()
                .map(|term| {
                    let df = idx
                        .files
                        .iter()
                        .filter(|f| {
                            f.lex_stats
                                .as_ref()
                                .map(|ls| ls.term_frequencies.contains_key(term.as_str()))
                                .unwrap_or(false)
                        })
                        .count()
                        .max(1);
                    (term.clone(), df)
                })
                .collect()
        })
        .unwrap_or_default();

    let mut grouped: BTreeMap<String, Vec<SearchMatch>> = BTreeMap::new();

    for item in matches {
        grouped.entry(item.path.clone()).or_default().push(item);
    }

    let candidates = grouped
        .into_iter()
        .map(|(path, mut matches)| {
            matches.sort_by_key(|item| item.line_number);
            build_candidate(path, matches, &profile, index, index_status, &lex_df)
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

    // Exact-phrase anchor protection: when a source/config file has an exact phrase
    // match in its content and no indexed-symbol boost (its high rank is purely from
    // lexical evidence), it must not be pushed out of the results by files that ranked
    // higher only because of index tier boosts.  Cap every competitor that lacks its
    // own exact-phrase evidence to just below the anchor's score.
    //
    // Applies for all query types when an index is active.  Files that carry their own
    // exact_phrase_match evidence are never capped — they earned their rank lexically.
    if index.is_some() {
        let best_exact_anchor_score = finalized
            .iter()
            .filter(|(c, raw)| {
                is_lexical_anchor(c, *raw)
                    && c.evidence
                        .iter()
                        .any(|e| e.evidence_type == "exact_phrase_match")
            })
            .map(|(c, _)| c.score)
            .fold(f64::NEG_INFINITY, f64::max);

        if best_exact_anchor_score.is_finite() {
            for (c, raw) in finalized.iter_mut() {
                let is_exact_anchor = is_lexical_anchor(c, *raw)
                    && c.evidence
                        .iter()
                        .any(|e| e.evidence_type == "exact_phrase_match");
                let has_own_exact_phrase = c
                    .evidence
                    .iter()
                    .any(|e| e.evidence_type == "exact_phrase_match");
                if !is_exact_anchor && !has_own_exact_phrase && c.score > best_exact_anchor_score {
                    c.score = round_score(best_exact_anchor_score - 0.01);
                }
            }
        }
    }

    finalized.sort_by(|left, right| {
        right
            .0
            .score
            .partial_cmp(&left.0.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                // Candidates whose tier came from filename/path shape evidence earned
                // it lexically; give them priority over candidates boosted solely by
                // index symbol definitions at the same score level.
                let left_shape = has_shape_tier_evidence(&left.0) as u8;
                let right_shape = has_shape_tier_evidence(&right.0) as u8;
                right_shape.cmp(&left_shape)
            })
            .then_with(|| {
                // Among candidates with the same score and shape tier origin, prefer
                // stronger phrase/lexical evidence (higher raw score).
                right
                    .1
                    .partial_cmp(&left.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| left.0.path.cmp(&right.0.path))
    });

    // Assign detail level based on score but do NOT strip data here.
    // Call `apply_tiered_density` on the result in main.rs before serialization.
    // Cap at CANDIDATE_ENUM_LIMIT regardless of score.
    finalized
        .into_iter()
        .take(CANDIDATE_ENUM_LIMIT)
        .map(|(mut c, _)| {
            c.detail_level = if c.score >= 0.70 {
                DetailLevel::Full
            } else if c.score >= 0.45 {
                DetailLevel::Medium
            } else if c.score >= 0.25 {
                DetailLevel::Minimal
            } else {
                DetailLevel::Enum
            };
            c
        })
        .collect()
}

/// Strip evidence/snippets from lower-detail candidates before JSON serialization.
/// Call this on the ranked list before building the final report.
pub fn apply_tiered_density(candidates: &mut Vec<FileCandidate>) {
    for c in candidates.iter_mut() {
        match c.detail_level {
            DetailLevel::Full => {}
            DetailLevel::Medium => {
                c.evidence.truncate(2);
            }
            DetailLevel::Minimal => {
                c.snippets.clear();
                c.evidence.truncate(1);
            }
            DetailLevel::Enum => {
                c.snippets.clear();
                c.evidence.clear();
                c.line_ranges.clear();
            }
        }
    }
}

/// Build a deduplicated vocabulary list from the top candidates' symbol names.
/// Returns the top N unique symbol names that matched the query, ordered by
/// their rank position (earlier-ranked candidates contribute first).
pub fn build_vocabulary(candidates: &[FileCandidate], limit: usize) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    let mut vocab = Vec::new();
    for candidate in candidates.iter().take(CANDIDATE_LIMIT) {
        for sym in &candidate.symbols {
            if seen.insert(sym.name.clone()) {
                vocab.push(sym.name.clone());
                if vocab.len() >= limit {
                    return vocab;
                }
            }
        }
    }
    vocab
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
    lex_tokens: Vec<String>,
    identifier_like: bool,
    /// Non-empty when the raw query is a single identifier (no spaces) that split
    /// into 2+ tokens — used to emit transparent "identifier expansion" evidence.
    expansion_tokens: Vec<String>,
}

impl QueryProfile {
    fn new(query: &str) -> Self {
        let raw = query.trim().to_string();
        let normalized_phrase = normalize_phrase(&raw);
        let squashed_phrase = squash_identifier(&normalized_phrase);
        let mut terms = tokenize_terms(&raw);
        terms.sort();
        terms.dedup();
        let lex_tokens = tokenize_lexical(&raw);
        let identifier_like = is_identifier_like(&raw, &terms);
        let expansion_tokens = if identifier_like && !raw.contains(' ') && terms.len() >= 2 {
            terms.clone()
        } else {
            vec![]
        };

        Self {
            normalized_phrase,
            squashed_phrase,
            terms,
            lex_tokens,
            identifier_like,
            expansion_tokens,
        }
    }
}

fn build_candidate(
    path: String,
    matches: Vec<SearchMatch>,
    profile: &QueryProfile,
    index: Option<&index::RepoIndex>,
    index_status: &str,
    lex_df: &HashMap<String, usize>,
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
    let best_exact_cluster = select_exact_phrase_cluster(&clusters, profile);
    let best_near_cluster = if best_exact_cluster.is_none() {
        select_near_phrase_cluster(&clusters, profile)
    } else {
        None
    };

    let mut evidence = Vec::new();
    let mut budget = ScoreBudget::new();
    let mut matched_terms = BTreeSet::new();

    for token_match in collect_token_matches(profile, &path_tokens, &file_name_tokens, &matches) {
        budget.lexical += token_match.score;
        if let Some(evidence_item) = token_match.evidence {
            evidence.push(evidence_item);
        }
        matched_terms.insert(token_match.term);
    }

    if let Some((boost, _, evidence_item)) =
        filename_shape_boost(profile, &path_tokens, &file_name_tokens)
    {
        budget.filename_shape += boost;
        evidence.push(evidence_item);
    }

    if let Some(cluster) = best_exact_cluster {
        // Use only the cluster's own content to decide fixture status — not the whole
        // file. file_has_test_signals was too broad: it penalised definition files that
        // also happen to contain a test module, causing the actual `pub struct Foo {`
        // cluster to be treated as a fixture even though it contains no assert/mock/test
        // keywords. cluster.is_fixture_like() checks the matched lines themselves.
        let fixture_like = cluster.is_fixture_like();
        let phrase_boost = exact_phrase_boost(&role, profile, fixture_like);
        budget.phrase += phrase_boost;
        evidence.push(Evidence {
            evidence_type: "exact_phrase_match".to_string(),
            detail: format!(
                "matched exact phrase in lines {}-{}",
                cluster.start_line, cluster.end_line
            ),
        });
        if fixture_like {
            budget.phrase -= 0.45;
            evidence.push(Evidence {
                evidence_type: "fixture_like_match".to_string(),
                detail: "exact phrase appears in assertion or fixture-like text".to_string(),
            });
        }
    } else if let Some(cluster) = best_near_cluster {
        budget.phrase += near_phrase_boost(&role, profile);
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

    // Identifier expansion evidence: when the query was a single identifier
    // (camelCase/snake_case) and its split tokens matched the filename or path,
    // emit a transparent "Why:" line so the user sees why this file ranked.
    if !profile.expansion_tokens.is_empty() {
        let matched: Vec<&str> = profile
            .expansion_tokens
            .iter()
            .filter(|et| {
                file_name_tokens.iter().any(|ft| token_matches_term(et, ft))
                    || path_tokens.iter().any(|pt| token_matches_term(et, pt))
            })
            .map(String::as_str)
            .collect();
        if matched.len() >= 2 {
            evidence.push(Evidence {
                evidence_type: "identifier_expansion".to_string(),
                detail: format!("identifier expansion matched {}", matched.join(" + ")),
            });
        }
    }

    budget.role += apply_role_weight(&role, &mut evidence);

    if let Some(index) = index {
        let (sym_def_score, ref_score, _) = apply_index_evidence(
            &normalized_path,
            &role,
            profile,
            index,
            index_status,
            &mut evidence,
        );
        budget.symbol_def += sym_def_score;
        budget.reference += ref_score;
        budget.lexical += apply_lex_score(
            &normalized_path,
            &profile.lex_tokens,
            lex_df,
            index,
            index_status,
            &mut evidence,
        );
    }

    let snippets = build_snippets(&clusters, profile);
    if !snippets.is_empty() {
        budget.lexical += 0.01 * snippets.len().min(3) as f64;
    }

    if !matches.is_empty() {
        let line_ranges = compress_line_ranges(&matches);
        let lines = format_line_ranges(&line_ranges);
        budget.lexical += 0.03 + (clusters.len().min(4) as f64 * 0.01);
        evidence.push(Evidence {
            evidence_type: "rg_match".to_string(),
            detail: format!("matched on lines {lines}"),
        });
    }

    if !profile.terms.is_empty() && !matched_terms.is_empty() {
        // Ratio-squared coverage: rewards files that match ALL query terms over files
        // that match most. For a 4-term query: 1/4→0.010, 2/4→0.040, 3/4→0.089,
        // 4/4→0.174 (+ 0.026 full-coverage bonus = 0.200 max, still within lex cap).
        let total_terms = profile.terms.len() as f64;
        let matched_count = matched_terms.len() as f64;
        let ratio = matched_count / total_terms;
        let coverage_score = ratio * ratio * 0.17;
        let full_bonus = if matched_terms.len() == profile.terms.len() {
            0.03
        } else {
            0.0
        };
        budget.lexical += coverage_score + full_bonus;
        evidence.push(Evidence {
            evidence_type: "query_term_coverage".to_string(),
            detail: format!(
                "matched {} of {} query terms",
                matched_terms.len(),
                profile.terms.len()
            ),
        });
    }

    // Fixture-like cap: force phrase budget to go deeply negative so total stays low
    if evidence
        .iter()
        .any(|item| item.evidence_type == "fixture_like_match")
    {
        budget.phrase = budget.phrase.min(-0.20);
    }

    // Deduplicate evidence by (type, detail) before final assembly
    let mut seen_evidence = std::collections::BTreeSet::new();
    evidence.retain(|e| seen_evidence.insert((e.evidence_type.clone(), e.detail.clone())));

    // God-file dampening: very large files mention every concept and dominate via BM25
    // even when a focused smaller file is the real answer.  Files >3× the repo median
    // get a proportional score discount so they don't crowd out targeted files.
    let size_dampen = if let Some(idx) = index {
        if let Some(file) = idx.files.iter().find(|f| f.path == normalized_path) {
            if let Some(lex_stats) = file.lex_stats.as_ref() {
                let avg = idx.stats.avg_doc_length;
                if avg > 0.0 {
                    let ratio = lex_stats.doc_length as f64 / avg;
                    if ratio > 10.0 {
                        0.72
                    } else if ratio > 5.0 {
                        0.83
                    } else if ratio > 3.0 {
                        0.91
                    } else {
                        1.0
                    }
                } else {
                    1.0
                }
            } else {
                1.0
            }
        } else {
            1.0
        }
    } else {
        1.0
    };

    let raw_score = (budget.total() * size_dampen).clamp(0.0, 1.0);
    let score = round_score(raw_score);
    let confidence = confidence_for(
        &role,
        profile,
        score,
        &evidence,
        matched_terms.len(),
        &snippets,
    );

    // Collect query-matching symbols defined in this file for the `symbols` field.
    let candidate_symbols: Vec<SymbolSummary> = index
        .map(|idx| {
            idx.symbols
                .iter()
                .filter(|s| {
                    s.file_path == normalized_path
                        && symbol_match_strength(&s.name, profile).is_some()
                })
                .map(|s| SymbolSummary {
                    name: s.name.clone(),
                    kind: s.kind.to_string(),
                    line: s.line_number,
                    parent_class: s.parent_class.clone(),
                })
                .collect()
        })
        .unwrap_or_default();

    RankedCandidate {
        candidate: FileCandidate {
            path,
            kind: "file".to_string(),
            role: role.to_string(),
            score,
            confidence,
            detail_level: DetailLevel::Full,
            line_ranges: compress_line_ranges(&matches),
            snippets,
            evidence,
            symbols: candidate_symbols,
        },
        matched_terms: matched_terms.len(),
        raw_score,
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
            // A short term that appears only as a fragment of a longer word ("rg"
            // inside "cargo", "cd" inside "discard") is incidental.  The same term
            // occurring as a bounded token — command name, path segment, symbol,
            // method call, or part of the exact phrase — is real evidence.
            // Only apply the reduced weight when the term is short (≤3 chars) in a
            // multi-term query (≥3 terms) and no snippet contains it as a bounded
            // occurrence; exact whole-token matches, quoted names, and symbol
            // components all keep full weight.
            let purely_embedded = profile.terms.len() >= 3
                && term.len() <= 3
                && !matches
                    .iter()
                    .any(|m| is_bounded_occurrence(&normalize_phrase(&m.snippet), term));
            let snippet_score = if purely_embedded {
                if profile.identifier_like {
                    0.04
                } else {
                    0.03
                }
            } else if profile.identifier_like {
                0.09
            } else {
                0.05
            };
            results.push(TokenMatch {
                term: term.clone(),
                score: snippet_score,
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

/// Returns true when `needle` appears in `haystack` bounded on both sides by
/// a non-alphanumeric character (or a string edge).  Covers command names,
/// path segments, method calls, quoted tokens, and space-separated words —
/// anything that is a discrete unit rather than a fragment of a longer word.
fn is_bounded_occurrence(haystack: &str, needle: &str) -> bool {
    haystack.match_indices(needle).any(|(i, _)| {
        let before = haystack[..i].chars().next_back();
        let after = haystack[i + needle.len()..].chars().next();
        before.map_or(true, |c| !c.is_alphanumeric())
            && after.map_or(true, |c| !c.is_alphanumeric())
    })
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

fn apply_role_weight(role: &FileRole, evidence: &mut Vec<Evidence>) -> f64 {
    match role {
        FileRole::Source => {
            evidence.push(Evidence {
                evidence_type: "source_role".to_string(),
                detail: "path suggests source-like file role".to_string(),
            });
            0.06
        }
        FileRole::Test => {
            evidence.push(Evidence {
                evidence_type: "test_role".to_string(),
                detail: "path suggests test file role".to_string(),
            });
            0.03
        }
        FileRole::Doc => {
            evidence.push(Evidence {
                evidence_type: "doc_role".to_string(),
                detail: "path suggests documentation file role".to_string(),
            });
            0.02
        }
        FileRole::Config => {
            evidence.push(Evidence {
                evidence_type: "config_role".to_string(),
                detail: "path suggests configuration file role".to_string(),
            });
            0.04
        }
        FileRole::Lockfile => {
            evidence.push(Evidence {
                evidence_type: "lockfile_role".to_string(),
                detail: "path suggests lockfile or dependency snapshot".to_string(),
            });
            -0.10
        }
        FileRole::Generated => {
            evidence.push(Evidence {
                evidence_type: "generated_role".to_string(),
                detail: "path suggests generated or build output".to_string(),
            });
            -0.05
        }
        FileRole::Other => 0.0,
    }
}

/// Returns (symbol_def_score, reference_score, tier) for budget routing.
/// Evidence items are inserted into the caller's evidence vec in priority order.
fn apply_index_evidence(
    path: &str,
    role: &FileRole,
    profile: &QueryProfile,
    index: &index::RepoIndex,
    index_status: &str,
    evidence: &mut Vec<Evidence>,
) -> (f64, f64, usize) {
    let scale = index_boost_scale(index_status);
    if scale <= 0.0 {
        return (0.0, 0.0, 0);
    }

    let mut def_boosts: Vec<IndexSignal> = Vec::new();
    let mut ref_boosts: Vec<IndexSignal> = Vec::new();
    let mut seen = BTreeSet::new();

    for symbol in index
        .symbols
        .iter()
        .filter(|symbol| symbol.file_path == path)
    {
        if let Some(signal) = symbol_definition_signal(symbol, profile, role, scale) {
            let key = format!("definition:{}:{}", symbol.name, symbol.line_number);
            if seen.insert(key) {
                def_boosts.push(signal);
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
                ref_boosts.push(signal);
            }
        }
    }

    for edge in index
        .edges
        .iter()
        .filter(|edge| edge.from == path || edge.to == path)
    {
        if let Some(signal) = edge_signal(edge, path, scale, profile, index) {
            let key = format!("edge:{}:{}:{}", edge.edge_type, edge.from, edge.to);
            if seen.insert(key) {
                ref_boosts.push(signal);
            }
        }
    }

    // Sort each group by tier desc then score desc; keep top 2 each for evidence
    let sort_signals = |boosts: &mut Vec<IndexSignal>| {
        boosts.sort_by(|l, r| {
            r.tier
                .cmp(&l.tier)
                .then_with(|| {
                    r.score
                        .partial_cmp(&l.score)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| {
                    index_confidence_priority(l.confidence)
                        .cmp(&index_confidence_priority(r.confidence))
                })
        });
    };
    sort_signals(&mut def_boosts);
    sort_signals(&mut ref_boosts);

    let index_tier = def_boosts
        .iter()
        .chain(ref_boosts.iter())
        .map(|s| s.tier)
        .max()
        .unwrap_or(0);

    let def_score: f64 = def_boosts.iter().take(2).map(|s| s.score).sum();
    let ref_score: f64 = ref_boosts.iter().take(2).map(|s| s.score).sum();

    // Insert evidence (highest priority first, at front of list)
    for signal in def_boosts.into_iter().take(2).rev() {
        evidence.insert(0, signal.evidence);
    }
    for signal in ref_boosts.into_iter().take(2).rev() {
        evidence.insert(0, signal.evidence);
    }

    (def_score, ref_score, index_tier)
}

fn apply_lex_score(
    path: &str,
    lex_tokens: &[String],
    lex_df: &HashMap<String, usize>,
    index: &index::RepoIndex,
    index_status: &str,
    evidence: &mut Vec<Evidence>,
) -> f64 {
    let scale = index_boost_scale(index_status);
    if scale <= 0.0 || lex_tokens.is_empty() {
        return 0.0;
    }

    let avg_doc_len = index.stats.avg_doc_length;
    let n = index.stats.lex_file_count;
    if avg_doc_len <= 0.0 || n == 0 {
        return 0.0;
    }

    let Some(file) = index.files.iter().find(|f| f.path == path) else {
        return 0.0;
    };
    let Some(lex_stats) = file.lex_stats.as_ref() else {
        return 0.0;
    };
    if lex_stats.doc_length == 0 {
        return 0.0;
    }

    let k1 = 1.5_f64;
    let b = 0.75_f64;
    let dl = lex_stats.doc_length as f64;
    let n_f = n as f64;

    let mut raw_score = 0.0_f64;
    let mut matched: Vec<&str> = Vec::new();

    for term in lex_tokens {
        let tf = match lex_stats.term_frequencies.get(term.as_str()) {
            Some(&tf) if tf > 0 => tf as f64,
            _ => continue,
        };

        let df = *lex_df.get(term).unwrap_or(&1) as f64;
        let idf = ((n_f - df + 0.5) / (df + 0.5) + 1.0).ln().max(0.0);
        let tf_norm = tf * (k1 + 1.0) / (tf + k1 * (1.0 - b + b * dl / avg_doc_len));

        raw_score += idf * tf_norm;
        matched.push(term);
    }

    if raw_score <= 0.0 || matched.is_empty() {
        return 0.0;
    }

    // Normalize to [0..1]: max per term is (k1+1) * ln(n+1), scaled to 0.15 max contribution.
    let max_per_term = (k1 + 1.0) * (n_f + 1.0).ln().max(1.0);
    let max_possible = lex_tokens.len() as f64 * max_per_term;
    let normalized = (raw_score / max_possible.max(1.0)).min(1.0);
    let contribution = (normalized * 0.15 * scale).max(0.0);

    if contribution < 0.01 {
        return 0.0;
    }

    if contribution >= 0.04 {
        // Insert right after the last indexed signal so this evidence is visible
        // in the "Why:" line (which shows the first 4 items).
        let insert_pos = evidence
            .iter()
            .rposition(|e| {
                e.evidence_type == "indexed_symbol_definition"
                    || e.evidence_type == "indexed_symbol_reference"
                    || e.evidence_type == "indexed_edge"
            })
            .map(|p| p + 1)
            .unwrap_or(0);
        evidence.insert(
            insert_pos,
            Evidence {
                evidence_type: "lexical_score".to_string(),
                detail: format!("lexical score matched terms: {}", matched.join(", ")),
            },
        );
    }

    contribution
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
    // Base values are intentionally below the sym_def cap (0.35) so that a file
    // with 2 exact definitions clearly outscores a file with 1, and so that a
    // focused filename-match file (shape=0.40) outranks a symbol-hub file with
    // only 1 exact definition (sym_def≤0.18).
    //
    // Exact Source identifier: 0.14+0.03=0.17  → 2 defs=0.34, capped at 0.35
    // Exact Source non-id:     0.13+0.03=0.16  → 2 defs=0.32, capped at 0.35
    // Strong:  0.07 → multiple needed to approach cap
    // Loose:   0.04 → even 5 Loose only reach 0.20
    let (base, tier, confidence) = match strength {
        SymbolMatchStrength::Exact => {
            let base = match role {
                FileRole::Source => {
                    if profile.identifier_like {
                        0.14
                    } else {
                        0.13
                    }
                }
                FileRole::Doc => 0.08,
                FileRole::Test => 0.10,
                FileRole::Config => 0.09,
                FileRole::Lockfile => 0.04,
                FileRole::Generated => 0.02,
                FileRole::Other => 0.10,
            };
            let tier = if matches!(role, FileRole::Source) {
                5
            } else {
                4
            };
            (base, tier, Confidence::High)
        }
        SymbolMatchStrength::Strong => (0.07, 3, Confidence::Medium),
        SymbolMatchStrength::Loose => (0.04, 2, Confidence::Low),
    };
    let mut score = base * scale;

    if score <= 0.0 {
        return None;
    }

    // Source-file bonus for exact definitions
    if matches!(strength, SymbolMatchStrength::Exact) && matches!(role, FileRole::Source) {
        score += 0.03 * scale;
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
    // Call-site references (target_file=None, confidence=Inferred) are stored for
    // used_by display but must NOT influence ranking — they match any symbol with
    // that name in the repo and would boost every calling file, not just relevant ones.
    if reference.target_file.is_none() && reference.confidence == EdgeConfidence::Inferred {
        return None;
    }

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

fn edge_signal(
    edge: &IndexedEdge,
    path: &str,
    scale: f64,
    profile: &QueryProfile,
    index: &index::RepoIndex,
) -> Option<IndexSignal> {
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
    // Query-aware multiplier: if the other endpoint's path/symbols match the query,
    // give a stronger boost so relevant graph connections surface. same_area stays
    // flat — it's structural co-location, not a meaningful dependency signal.
    let relevance = if edge.edge_type != "same_area" {
        let other_path = if edge.from == path {
            &edge.to
        } else {
            &edge.from
        };
        compute_query_edge_relevance(other_path, profile, index)
    } else {
        1.0
    };
    let score = (base + directional_bonus) * scale * relevance;
    if score <= 0.0 {
        return None;
    }

    let detail = if relevance > 1.0 {
        let other_path = if edge.from == path {
            &edge.to
        } else {
            &edge.from
        };
        format!(
            "connected to query-relevant {} via {}",
            other_path, edge.edge_type
        )
    } else {
        format!("indexed edge {} {}", edge.edge_type, edge.reason)
    };

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
            detail,
        },
    })
}

/// Returns a score multiplier (≥1.0) for an edge whose other endpoint's
/// path or symbols are relevant to the query.  Kept modest so edges never
/// swamp direct text/symbol evidence.
fn compute_query_edge_relevance(
    other_path: &str,
    profile: &QueryProfile,
    index: &index::RepoIndex,
) -> f64 {
    if profile.terms.is_empty() {
        return 1.0;
    }
    let other_tokens = tokenize_terms(other_path);
    let term_hits = profile
        .terms
        .iter()
        .filter(|t| other_tokens.iter().any(|ot| token_matches_term(t, ot)))
        .count();
    if term_hits == 0 {
        return 1.0;
    }
    let has_symbol = index
        .symbols
        .iter()
        .any(|s| s.file_path == other_path && symbol_match_strength(&s.name, profile).is_some());
    if has_symbol {
        1.5
    } else if term_hits >= profile.terms.len() {
        1.3
    } else {
        1.1
    }
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
    matched_terms: usize,
    raw_score: f64,
}

fn finalize_ranked_candidate(ranked: RankedCandidate) -> (FileCandidate, f64) {
    // Score is already budget-capped in [0.0, 1.0] from build_candidate.
    // Tier is used only for sort-order tiebreaking (see sort comparator in rank_with_index).
    // Confidence is set by confidence_for() based on evidence quality; no override here.
    (ranked.candidate, ranked.raw_score)
}

/// Returns true when the candidate has filename or path shape evidence, meaning
/// its high tier came from lexical filename matching rather than from index symbol
/// definitions alone.  Used to give filename-matched candidates priority over pure
/// symbol-hub files at the same score level.
fn has_shape_tier_evidence(candidate: &FileCandidate) -> bool {
    candidate.evidence.iter().any(|e| {
        matches!(
            e.evidence_type.as_str(),
            "filename_shape_match" | "path_shape_match"
        )
    })
}

/// Returns true when a candidate is a strong pure-lexical anchor that should
/// not be displaced by index tier boosts.
///
/// Qualifying conditions (all must hold):
/// - Phrase evidence (exact or high-confidence near phrase in source/config)
/// - Source or config role
/// - Not fixture-like
/// - No indexed evidence (the candidate's score is driven by lexical signals only)
/// - Raw score above the phrase-strength threshold
///
/// Because near-phrase is weaker evidence than exact-phrase, it requires a
/// higher raw-score bar to qualify.
fn is_lexical_anchor(candidate: &FileCandidate, raw_score: f64) -> bool {
    let has_exact = candidate
        .evidence
        .iter()
        .any(|e| e.evidence_type == "exact_phrase_match");
    let has_near = candidate
        .evidence
        .iter()
        .any(|e| e.evidence_type == "near_phrase_match");
    let not_fixture = !candidate
        .evidence
        .iter()
        .any(|e| e.evidence_type == "fixture_like_match");
    let is_source_or_config = matches!(candidate.role.as_str(), "source" | "config");
    // Files that received indexed_symbol_definition or indexed_symbol_reference
    // boosts already had their rank corrected by the symbol index; they are not
    // pure lexical anchors.  indexed_edge signals are structural and weaker —
    // a file can still qualify as an anchor even if it carries edge evidence.
    let has_symbol_index = candidate.evidence.iter().any(|e| {
        matches!(
            e.evidence_type.as_str(),
            "indexed_symbol_definition" | "indexed_symbol_reference"
        )
    });

    if !not_fixture || !is_source_or_config || has_symbol_index {
        return false;
    }

    // Exact phrase in source/config with no index boost is a strong anchor.
    if has_exact && raw_score >= 0.50 {
        return true;
    }

    // Near phrase requires a higher raw-score threshold — it is weaker evidence.
    has_near && raw_score >= 0.80
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
        EdgeConfidence, FileLexStats, FileRole as IndexFileRole, IndexStats, IndexedEdge,
        IndexedFile, IndexedSymbolReference, ReferenceContext, RepoIndex,
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
            end_line: None,

            parent_class: None,        }
    }

    fn indexed_file(path: &str, role: IndexFileRole) -> IndexedFile {
        IndexedFile {
            path: path.to_string(),
            role,
            size_bytes: None,
            modified_unix: None,
            content_hash: None,
            lex_stats: None,
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
                ..Default::default()
            },
                dep_imports: vec![],
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

    fn make_lex_index(files: Vec<IndexedFile>, avg_doc_length: f64) -> RepoIndex {
        let lex_file_count = files.iter().filter(|f| f.lex_stats.is_some()).count();
        RepoIndex {
            schema_version: crate::index::INDEX_SCHEMA_VERSION,
            repo_root: "C:/repo".to_string(),
            repo_rev: Some("abc".to_string()),
            indexed_at_unix: 1,
            files,
            symbols: vec![],
            symbol_references: vec![],
            edges: vec![],
            stats: IndexStats {
                file_count: 2,
                role_counts: BTreeMap::from([(IndexFileRole::Source, 2)]),
                symbol_count: 0,
                symbol_kind_counts: BTreeMap::new(),
                symbol_reference_count: 0,
                connection_count: 0,
                lex_file_count,
                avg_doc_length,
            },
                dep_imports: vec![],
        }
    }

    #[test]
    fn lexical_score_boosts_file_with_repeated_query_terms() {
        let matches = vec![
            match_item("src/downloader.rs", 10, "fn download_progress() {}"),
            match_item("src/utils.rs", 5, "// some download and progress utility"),
        ];

        // 10 files total — only downloader.rs and utils.rs contain "download"/"progress".
        // This gives meaningful IDF and a contribution well above the evidence threshold.
        let mut bg_files: Vec<IndexedFile> = (0..8)
            .map(|i| IndexedFile {
                path: format!("src/module{i}.rs"),
                role: IndexFileRole::Source,
                size_bytes: None,
                modified_unix: None,
                content_hash: None,
                lex_stats: Some(FileLexStats {
                    doc_length: 500,
                    term_frequencies: BTreeMap::from([("result".to_string(), 10u32)]),
                }),
            })
            .collect();

        let mut files = vec![
            IndexedFile {
                path: "src/downloader.rs".to_string(),
                role: IndexFileRole::Source,
                size_bytes: None,
                modified_unix: None,
                content_hash: None,
                lex_stats: Some(FileLexStats {
                    doc_length: 500,
                    term_frequencies: BTreeMap::from([
                        ("download".to_string(), 28u32),
                        ("progress".to_string(), 18u32),
                    ]),
                }),
            },
            IndexedFile {
                path: "src/utils.rs".to_string(),
                role: IndexFileRole::Source,
                size_bytes: None,
                modified_unix: None,
                content_hash: None,
                lex_stats: Some(FileLexStats {
                    doc_length: 500,
                    term_frequencies: BTreeMap::from([
                        ("download".to_string(), 1u32),
                        ("progress".to_string(), 1u32),
                    ]),
                }),
            },
        ];
        files.append(&mut bg_files);

        let index = make_lex_index(files, 500.0);

        let candidates = rank_with_index(
            "download progress",
            matches,
            Some(&index),
            "fresh",
            &FindFilters::default(),
        );

        assert_eq!(candidates.first().unwrap().path, "src/downloader.rs");
        assert!(candidates
            .first()
            .unwrap()
            .evidence
            .iter()
            .any(|e| e.evidence_type == "lexical_score"));
        let lex_ev = candidates
            .first()
            .unwrap()
            .evidence
            .iter()
            .find(|e| e.evidence_type == "lexical_score")
            .unwrap();
        assert!(
            lex_ev.detail.contains("download") || lex_ev.detail.contains("progress"),
            "evidence detail should name the matched terms"
        );
    }

    #[test]
    fn exact_symbol_definition_beats_lexical_noise() {
        // The symbol-definition file has weak lex stats, but the definition signal wins.
        let matches = vec![
            match_item("src/download.rs", 11, "pub struct DownloadProgress {"),
            match_item(
                "src/tracker.rs",
                22,
                "fn update_download_progress(item: &DownloadProgress) {}",
            ),
        ];

        let index = RepoIndex {
            schema_version: crate::index::INDEX_SCHEMA_VERSION,
            repo_root: "C:/repo".to_string(),
            repo_rev: Some("abc".to_string()),
            indexed_at_unix: 1,
            files: vec![
                IndexedFile {
                    path: "src/download.rs".to_string(),
                    role: IndexFileRole::Source,
                    size_bytes: None,
                    modified_unix: None,
                    content_hash: None,
                    lex_stats: Some(FileLexStats {
                        doc_length: 300,
                        term_frequencies: BTreeMap::from([
                            ("download".to_string(), 5u32),
                            ("progress".to_string(), 3u32),
                        ]),
                    }),
                },
                IndexedFile {
                    path: "src/tracker.rs".to_string(),
                    role: IndexFileRole::Source,
                    size_bytes: None,
                    modified_unix: None,
                    content_hash: None,
                    lex_stats: Some(FileLexStats {
                        doc_length: 300,
                        term_frequencies: BTreeMap::from([
                            ("download".to_string(), 30u32),
                            ("progress".to_string(), 25u32),
                        ]),
                    }),
                },
            ],
            symbols: vec![crate::types::IndexedSymbol {
                name: "DownloadProgress".to_string(),
                kind: crate::types::SymbolKind::Struct,
                file_path: "src/download.rs".to_string(),
                line_number: 11,
                visibility: crate::types::Visibility::Public,
                signature: Some("pub struct DownloadProgress {".to_string()),
                end_line: None,

            parent_class: None,            }],
            symbol_references: vec![],
            edges: vec![],
            stats: IndexStats {
                file_count: 2,
                role_counts: BTreeMap::from([(IndexFileRole::Source, 2)]),
                symbol_count: 1,
                symbol_kind_counts: BTreeMap::new(),
                symbol_reference_count: 0,
                connection_count: 0,
                lex_file_count: 2,
                avg_doc_length: 300.0,
            },
                dep_imports: vec![],
        };

        let candidates = rank_with_index(
            "DownloadProgress",
            matches,
            Some(&index),
            "fresh",
            &FindFilters::default(),
        );

        assert_eq!(candidates.first().unwrap().path, "src/download.rs");
        assert!(candidates
            .first()
            .unwrap()
            .evidence
            .iter()
            .any(|e| e.evidence_type == "indexed_symbol_definition"));
    }

    #[test]
    fn lexical_score_absent_when_index_missing() {
        let matches = vec![match_item("src/downloader.rs", 10, "download progress")];

        let candidates = rank_with_index(
            "download progress",
            matches,
            None,
            "not_applicable",
            &FindFilters::default(),
        );

        assert!(!candidates
            .first()
            .unwrap()
            .evidence
            .iter()
            .any(|e| e.evidence_type == "lexical_score"));
    }

    #[test]
    fn identifier_expansion_emits_evidence_for_camel_case_query() {
        // Query "downloadProgress" splits into [download, progress].
        // The file named download_progress.rs should match via expansion and
        // emit identifier_expansion evidence.
        let matches = vec![
            match_item("src/download_progress.rs", 5, "fn track() {}"),
            match_item("src/utils.rs", 10, "// downloadProgress helper"),
        ];

        let candidates = rank_with_index(
            "downloadProgress",
            matches,
            None,
            "not_applicable",
            &FindFilters::default(),
        );

        let top = candidates.first().unwrap();
        assert_eq!(top.path, "src/download_progress.rs");
        assert!(
            top.evidence
                .iter()
                .any(|e| e.evidence_type == "identifier_expansion"),
            "expected identifier_expansion evidence for camelCase query"
        );
        let ev = top
            .evidence
            .iter()
            .find(|e| e.evidence_type == "identifier_expansion")
            .unwrap();
        assert!(
            ev.detail.contains("download") && ev.detail.contains("progress"),
            "expansion evidence should name the matched tokens"
        );
    }

    #[test]
    fn graph_boost_helps_files_connected_to_query_relevant_target() {
        // src/main.rs imports src/download_manager.rs (which has symbols matching
        // the query). src/main.rs should get a graph boost and rank above src/other.rs
        // which has a same-relevance rg match but no edge to a query-relevant file.
        let matches = vec![
            match_item("src/main.rs", 5, "use download_manager;"),
            match_item("src/other.rs", 12, "use download_manager;"),
        ];

        let index = RepoIndex {
            schema_version: crate::index::INDEX_SCHEMA_VERSION,
            repo_root: "C:/repo".to_string(),
            repo_rev: Some("abc".to_string()),
            indexed_at_unix: 1,
            files: vec![
                indexed_file("src/main.rs", IndexFileRole::Source),
                indexed_file("src/other.rs", IndexFileRole::Source),
                indexed_file("src/download_manager.rs", IndexFileRole::Source),
            ],
            symbols: vec![indexed_symbol(
                "DownloadManager",
                "src/download_manager.rs",
                1,
            )],
            symbol_references: vec![],
            edges: vec![
                IndexedEdge {
                    edge_type: "imports".to_string(),
                    from: "src/main.rs".to_string(),
                    to: "src/download_manager.rs".to_string(),
                    confidence: EdgeConfidence::Extracted,
                    reason: "imports download_manager".to_string(),
                },
                IndexedEdge {
                    edge_type: "same_area".to_string(),
                    from: "src/other.rs".to_string(),
                    to: "src/download_manager.rs".to_string(),
                    confidence: EdgeConfidence::Extracted,
                    reason: "shared source area src".to_string(),
                },
            ],
            stats: IndexStats {
                file_count: 3,
                role_counts: BTreeMap::from([(IndexFileRole::Source, 3)]),
                symbol_count: 1,
                symbol_kind_counts: BTreeMap::new(),
                symbol_reference_count: 0,
                connection_count: 1,
                ..Default::default()
            },
                dep_imports: vec![],
        };

        let candidates = rank_with_index(
            "DownloadManager",
            matches,
            Some(&index),
            "fresh",
            &FindFilters::default(),
        );

        let main_score = candidates
            .iter()
            .find(|c| c.path == "src/main.rs")
            .map(|c| c.score)
            .unwrap_or(0.0);
        let other_score = candidates
            .iter()
            .find(|c| c.path == "src/other.rs")
            .map(|c| c.score)
            .unwrap_or(0.0);

        assert!(
            main_score > other_score,
            "imports edge to query-relevant file should beat same_area edge (main={main_score} other={other_score})"
        );
        // Graph evidence should be present for main.rs
        let main = candidates.iter().find(|c| c.path == "src/main.rs").unwrap();
        assert!(
            main.evidence
                .iter()
                .any(|e| e.evidence_type == "indexed_edge"),
            "main.rs should have indexed_edge evidence from the graph boost"
        );
    }

    #[test]
    fn same_area_edge_stays_weak_regardless_of_query_relevance() {
        // A same_area edge to a perfectly query-relevant file should not produce
        // a strong boost — same_area is structural co-location, not a real dependency.
        let matches = vec![
            match_item("src/consumer.rs", 10, "use progress;"),
            match_item("src/other.rs", 5, "use progress;"),
        ];

        let index = RepoIndex {
            schema_version: crate::index::INDEX_SCHEMA_VERSION,
            repo_root: "C:/repo".to_string(),
            repo_rev: Some("abc".to_string()),
            indexed_at_unix: 1,
            files: vec![
                indexed_file("src/consumer.rs", IndexFileRole::Source),
                indexed_file("src/other.rs", IndexFileRole::Source),
                indexed_file("src/progress.rs", IndexFileRole::Source),
            ],
            symbols: vec![indexed_symbol("Progress", "src/progress.rs", 1)],
            symbol_references: vec![],
            edges: vec![
                // consumer.rs imports progress.rs (strong edge to query-relevant target)
                IndexedEdge {
                    edge_type: "imports".to_string(),
                    from: "src/consumer.rs".to_string(),
                    to: "src/progress.rs".to_string(),
                    confidence: EdgeConfidence::Extracted,
                    reason: "imports progress".to_string(),
                },
                // other.rs has same_area with progress.rs (weak co-location only)
                IndexedEdge {
                    edge_type: "same_area".to_string(),
                    from: "src/other.rs".to_string(),
                    to: "src/progress.rs".to_string(),
                    confidence: EdgeConfidence::Extracted,
                    reason: "shared source area src".to_string(),
                },
            ],
            stats: IndexStats {
                file_count: 3,
                role_counts: BTreeMap::from([(IndexFileRole::Source, 3)]),
                symbol_count: 1,
                symbol_kind_counts: BTreeMap::new(),
                symbol_reference_count: 0,
                connection_count: 1,
                ..Default::default()
            },
                dep_imports: vec![],
        };

        let candidates = rank_with_index(
            "Progress",
            matches,
            Some(&index),
            "fresh",
            &FindFilters::default(),
        );

        let consumer_score = candidates
            .iter()
            .find(|c| c.path == "src/consumer.rs")
            .map(|c| c.score)
            .unwrap_or(0.0);
        let other_score = candidates
            .iter()
            .find(|c| c.path == "src/other.rs")
            .map(|c| c.score)
            .unwrap_or(0.0);

        assert!(
            consumer_score > other_score,
            "imports edge should outrank same_area edge even when both point to a query-relevant file \
             (consumer={consumer_score}, other={other_score})"
        );
    }

    #[test]
    fn definition_file_with_embedded_tests_beats_fixture_reference() {
        // Regression test for the agentgrep dogfood bug:
        // `agentgrep find "SearchResult"` was ranking types.rs #1 and the actual
        // definition file (search.rs) #4.
        //
        // Root cause: file_has_test_signals() was checking ALL rg matches in the file.
        // search.rs defines `pub struct SearchResult` AND has a test module with #[test]
        // annotations. The test-module match triggered the fixture penalty, capping
        // search.rs's score at 0.30 + tier_boost = 1.05, while types.rs (no test signals)
        // scored ~1.30 cleanly.
        //
        // Fix: fixture_like now comes only from cluster.is_fixture_like() — the matched
        // CONTENT, not the file as a whole. The definition cluster at line 11 does not
        // contain assert/mock/test keywords, so it is not penalised.
        let matches = vec![
            // The real definition — this is the cluster that matters:
            match_item("src/search.rs", 11, "pub struct SearchResult {"),
            // A match from the test module inside the same file:
            match_item("src/search.rs", 850, "#[test] fn it_returns_results() {}"),
            // Another file with the symbol only as a fixture string:
            match_item(
                "src/blast.rs",
                200,
                "build_report(&repo(), &index(), \"SearchResult\")",
            ),
        ];

        let index = repo_index(
            vec![indexed_symbol("SearchResult", "src/search.rs", 11)],
            vec![],
            vec![],
            vec![
                indexed_file("src/search.rs", IndexFileRole::Source),
                indexed_file("src/blast.rs", IndexFileRole::Source),
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
        assert!(
            candidates
                .first()
                .unwrap()
                .evidence
                .iter()
                .any(|e| e.evidence_type == "indexed_symbol_definition"),
            "definition file must show indexed_symbol_definition in evidence"
        );
        // Sanity: the blast.rs fixture reference must rank below the definition.
        let search_score = candidates
            .iter()
            .find(|c| c.path == "src/search.rs")
            .map(|c| c.score)
            .unwrap_or(0.0);
        let blast_score = candidates
            .iter()
            .find(|c| c.path == "src/blast.rs")
            .map(|c| c.score)
            .unwrap_or(0.0);
        assert!(
            search_score > blast_score,
            "definition (search.rs={search_score}) must outscore fixture reference (blast.rs={blast_score})"
        );
    }
}

