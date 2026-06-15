use std::collections::{BTreeMap, BTreeSet};

use crate::text::{normalize_phrase, shorten_snippet, squash_identifier, tokenize_terms};
use crate::types::{Confidence, Evidence, FileCandidate, LineRange, SearchMatch, Snippet};

pub const CANDIDATE_LIMIT: usize = 8;

pub fn rank(query: &str, matches: Vec<SearchMatch>) -> Vec<FileCandidate> {
    let profile = QueryProfile::new(query);
    let mut grouped: BTreeMap<String, Vec<SearchMatch>> = BTreeMap::new();

    for item in matches {
        grouped.entry(item.path.clone()).or_default().push(item);
    }

    let mut candidates = grouped
        .into_iter()
        .map(|(path, mut matches)| {
            matches.sort_by_key(|item| item.line_number);
            build_candidate(path, matches, &profile)
        })
        .collect::<Vec<_>>();

    candidates.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.path.cmp(&right.path))
    });

    candidates.truncate(CANDIDATE_LIMIT);
    candidates
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
    dependency_related: bool,
    error_like: bool,
}

impl QueryProfile {
    fn new(query: &str) -> Self {
        let raw = query.trim().to_string();
        let normalized_phrase = normalize_phrase(&raw);
        let squashed_phrase = squash_identifier(&normalized_phrase);
        let terms = tokenize_terms(&raw);
        let identifier_like = is_identifier_like(&raw, &terms);
        let dependency_related = is_dependency_related(&raw, &terms);
        let error_like = is_error_like(&raw, &terms);

        Self {
            normalized_phrase,
            squashed_phrase,
            terms,
            identifier_like,
            dependency_related,
            error_like,
        }
    }
}

fn build_candidate(
    path: String,
    matches: Vec<SearchMatch>,
    profile: &QueryProfile,
) -> FileCandidate {
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

    for token_match in collect_token_matches(profile, &path_tokens, &file_name_tokens, &matches) {
        score += token_match.score;
        if let Some(evidence_item) = token_match.evidence {
            evidence.push(evidence_item);
        }
        matched_terms.insert(token_match.term);
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

    apply_role_weight(&role, profile, &mut score, &mut evidence);

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

    if matched_terms.len() >= 2 {
        score += 0.04;
        evidence.push(Evidence {
            evidence_type: "multi_term_match".to_string(),
            detail: format!("matched {} query terms", matched_terms.len()),
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

    FileCandidate {
        path,
        kind: "file".to_string(),
        role: role.to_string(),
        score,
        confidence,
        line_ranges: compress_line_ranges(&matches),
        snippets,
        evidence,
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
        let filename_hit = file_name_tokens.iter().any(|token| token == term);
        let path_hit = path_tokens.iter().any(|token| token == term);
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

fn exact_phrase_boost(role: &FileRole, profile: &QueryProfile, fixture_like: bool) -> f64 {
    let mut boost = match role {
        FileRole::Source => {
            if profile.error_like {
                0.50
            } else if profile.identifier_like {
                0.42
            } else {
                0.34
            }
        }
        FileRole::Doc => {
            if profile.error_like {
                0.18
            } else if profile.identifier_like {
                0.12
            } else {
                0.24
            }
        }
        FileRole::Test => 0.18,
        FileRole::Config => 0.14,
        FileRole::Lockfile => 0.08,
        FileRole::Generated => 0.02,
        FileRole::Other => 0.20,
    };

    if fixture_like {
        boost -= 0.12;
    }

    boost
}

fn near_phrase_boost(role: &FileRole, profile: &QueryProfile) -> f64 {
    match role {
        FileRole::Source => {
            if profile.identifier_like {
                0.18
            } else {
                0.14
            }
        }
        FileRole::Doc => {
            if profile.error_like {
                0.04
            } else {
                0.08
            }
        }
        FileRole::Test => 0.08,
        FileRole::Config => 0.06,
        FileRole::Lockfile => 0.02,
        FileRole::Generated => 0.0,
        FileRole::Other => 0.08,
    }
}

fn apply_role_weight(
    role: &FileRole,
    profile: &QueryProfile,
    score: &mut f64,
    evidence: &mut Vec<Evidence>,
) {
    match role {
        FileRole::Source => {
            let boost = if profile.identifier_like {
                0.16
            } else if profile.error_like {
                0.12
            } else {
                0.08
            };
            *score += boost;
            evidence.push(Evidence {
                evidence_type: "source_role".to_string(),
                detail: "path suggests source implementation".to_string(),
            });
        }
        FileRole::Test => {
            let boost = if profile.identifier_like { 0.04 } else { 0.02 };
            *score += boost;
            evidence.push(Evidence {
                evidence_type: "test_role".to_string(),
                detail: "path suggests tests".to_string(),
            });
        }
        FileRole::Doc => {
            let delta = if profile.error_like {
                -0.12
            } else if profile.identifier_like {
                -0.10
            } else {
                0.02
            };
            *score += delta;
            evidence.push(Evidence {
                evidence_type: "doc_role".to_string(),
                detail: "path suggests documentation".to_string(),
            });
        }
        FileRole::Config => {
            let boost = if profile.dependency_related {
                0.08
            } else {
                0.03
            };
            *score += boost;
            evidence.push(Evidence {
                evidence_type: "config_role".to_string(),
                detail: "path suggests configuration".to_string(),
            });
        }
        FileRole::Lockfile => {
            let delta = if profile.dependency_related {
                0.04
            } else {
                -0.28
            };
            *score += delta;
            evidence.push(Evidence {
                evidence_type: "lockfile_role".to_string(),
                detail: if profile.dependency_related {
                    "lockfile is relevant to dependency-related query".to_string()
                } else {
                    "lockfile penalized for non-dependency query".to_string()
                },
            });
        }
        FileRole::Generated => {
            *score -= 0.35;
            evidence.push(Evidence {
                evidence_type: "generated_role".to_string(),
                detail: "path suggests generated or build output".to_string(),
            });
        }
        FileRole::Other => {}
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

    if has_strong_snippet
        || (profile.identifier_like && role_is_source && has_exact_phrase)
        || (profile.error_like && role_is_source && has_exact_phrase)
    {
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

fn is_dependency_related(raw: &str, terms: &[String]) -> bool {
    let lower = raw.to_lowercase();
    let keywords = [
        "dependency",
        "dependencies",
        "lock",
        "lockfile",
        "package",
        "version",
        "cargo",
        "npm",
        "yarn",
        "pnpm",
        "crate",
        "crates",
    ];

    lower.contains("lockfile")
        || terms.iter().any(|term| keywords.contains(&term.as_str()))
        || lower.contains("package-lock")
        || lower.contains("cargo.lock")
}

fn is_error_like(raw: &str, terms: &[String]) -> bool {
    let lower = raw.to_lowercase();
    lower.contains("not found")
        || lower.contains("error")
        || lower.contains("failed")
        || lower.contains("missing")
        || lower.contains("install")
        || terms.iter().any(|term| {
            matches!(
                term.as_str(),
                "error" | "failed" | "missing" | "install" | "not"
            )
        })
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

    fn match_item(path: &str, line_number: usize, snippet: &str) -> SearchMatch {
        SearchMatch {
            path: path.to_string(),
            line_number,
            snippet: snippet.to_string(),
        }
    }

    #[test]
    fn token_aware_path_matching_does_not_match_cargo_lock_for_rg() {
        let matches = vec![
            match_item("Cargo.lock", 1, "rg"),
            match_item("src/lib.rs", 4, "rg"),
        ];

        let candidates = rank("rg", matches);
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

        let candidates = rank("rg was not found", matches);
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

        let candidates = rank("rg was not found", matches);
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

        let candidates = rank("query", matches);
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

        let candidates = rank("line_ranges", matches);
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

        let candidates = rank("snippet", matches);
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

        let strong_candidate = rank("SearchReport", strong).remove(0);
        let weak_candidate = rank("search report", weak).remove(0);

        assert_eq!(strong_candidate.confidence, Confidence::High);
        assert_eq!(weak_candidate.confidence, Confidence::Low);
    }
}
