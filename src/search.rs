use anyhow::{anyhow, Context, Result};
use std::collections::{BTreeMap, HashSet};
use std::process::Command;

use crate::text::{normalize_phrase, tokenize_terms};
use crate::types::{SearchCoverage, SearchMatch};

pub const MATCH_LIMIT_PER_FILE: usize = 20;

#[derive(Debug)]
pub struct SearchResult {
    pub matches: Vec<SearchMatch>,
    pub coverage: SearchCoverage,
    pub match_limit_hit: bool,
}

pub fn run_with_index(
    repo_root: &std::path::Path,
    query: &str,
    index_used: bool,
    index_status: &str,
) -> Result<SearchResult> {
    let token_patterns = build_token_patterns(query);
    let exact_phrase = normalize_phrase(query);

    if token_patterns.is_empty() && exact_phrase.is_empty() {
        return Err(anyhow!("query must not be empty"));
    }

    let mut raw_match_count = 0usize;
    let mut matches = Vec::new();
    let mut seen = HashSet::new();
    let mut seen_files = HashSet::new();
    let mut raw_seen = HashSet::new();
    let mut raw_file_match_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut match_limit_hit = false;

    if !exact_phrase.is_empty() {
        collect_matches(
            repo_root,
            &[exact_phrase.clone()],
            &mut matches,
            &mut seen,
            &mut seen_files,
            &mut raw_seen,
            &mut raw_file_match_counts,
            &mut match_limit_hit,
            &mut raw_match_count,
        )?;
    }

    if !token_patterns.is_empty() {
        collect_matches(
            repo_root,
            &token_patterns,
            &mut matches,
            &mut seen,
            &mut seen_files,
            &mut raw_seen,
            &mut raw_file_match_counts,
            &mut match_limit_hit,
            &mut raw_match_count,
        )?;
    }

    let coverage = apply_index_metadata(
        SearchCoverage::new(raw_match_count, seen_files.len(), MATCH_LIMIT_PER_FILE),
        index_used,
        index_status,
    );

    Ok(SearchResult {
        matches,
        coverage,
        match_limit_hit,
    })
}

pub(crate) fn apply_index_metadata(
    mut coverage: SearchCoverage,
    index_used: bool,
    index_status: &str,
) -> SearchCoverage {
    coverage.index_used = index_used;
    coverage.index_status = index_status.to_string();
    coverage
}

fn collect_matches(
    repo_root: &std::path::Path,
    patterns: &[String],
    matches: &mut Vec<SearchMatch>,
    seen: &mut HashSet<String>,
    seen_files: &mut HashSet<String>,
    raw_seen: &mut HashSet<String>,
    raw_file_match_counts: &mut BTreeMap<String, usize>,
    match_limit_hit: &mut bool,
    raw_match_count: &mut usize,
) -> Result<()> {
    let output = run_rg(repo_root, patterns)?;

    for line in output.lines() {
        if let Some(parsed) = parse_match_line(line) {
            if is_capture_output_path(&parsed.path) {
                continue;
            }

            let raw_key = format!("{}:{}:{}", parsed.path, parsed.line_number, parsed.snippet);
            if raw_seen.insert(raw_key.clone()) {
                *raw_match_count += 1;
                seen_files.insert(parsed.path.clone());
                let count = raw_file_match_counts
                    .entry(parsed.path.clone())
                    .or_insert(0);
                *count += 1;
                if *count >= MATCH_LIMIT_PER_FILE {
                    *match_limit_hit = true;
                }
            }

            if seen.insert(raw_key) {
                matches.push(parsed);
            }
        }
    }

    Ok(())
}

fn run_rg(repo_root: &std::path::Path, patterns: &[String]) -> Result<String> {
    let mut cmd = Command::new("rg");
    cmd.current_dir(repo_root)
        .arg("--line-number")
        .arg("--no-heading")
        .arg("--color")
        .arg("never")
        .arg("--smart-case")
        .arg("--hidden")
        .arg("--max-count")
        .arg(MATCH_LIMIT_PER_FILE.to_string())
        .arg("--glob")
        .arg("!**/.git/**")
        .arg("--glob")
        .arg("!**/target/**")
        .arg("--glob")
        .arg("!**/node_modules/**")
        .arg("--glob")
        .arg("!**/dist/**")
        .arg("--glob")
        .arg("!**/build/**")
        .arg("--glob")
        .arg("!**/manual-test/**");

    for pattern in patterns {
        cmd.arg("-F").arg("-e").arg(pattern);
    }

    let output = match cmd.output() {
        Ok(output) => output,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Err(anyhow!(
                "rg was not found on PATH. Install ripgrep and try again."
            ));
        }
        Err(err) => return Err(err).context("failed to run rg"),
    };

    if !output.status.success() && output.status.code() != Some(1) {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            return Err(anyhow!("rg failed"));
        }
        return Err(anyhow!("rg failed: {stderr}"));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn build_token_patterns(query: &str) -> Vec<String> {
    let mut patterns = Vec::new();

    for term in tokenize_terms(query) {
        if !patterns.contains(&term) {
            patterns.push(term);
        }
    }

    let trimmed = normalize_phrase(query);
    if !trimmed.is_empty()
        && trimmed.len() > 1
        && !patterns.iter().any(|pattern| pattern == &trimmed)
    {
        patterns.push(trimmed);
    }

    patterns
}

fn parse_match_line(line: &str) -> Option<SearchMatch> {
    let mut parts = line.splitn(3, ':');
    let path = parts.next()?.replace('\\', "/");
    let line_number = parts.next()?.parse().ok()?;
    let snippet = parts.next()?.to_string();

    Some(SearchMatch {
        path,
        line_number,
        snippet,
    })
}

fn is_capture_output_path(path: &str) -> bool {
    path.starts_with("manual-test/") || path.contains("/manual-test/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_terms_and_phrase() {
        let patterns = build_token_patterns("auth redirect");
        assert_eq!(patterns, vec!["auth", "redirect", "auth redirect"]);
    }

    #[test]
    fn splits_camel_case_queries() {
        let patterns = build_token_patterns("SearchReport");
        assert_eq!(patterns, vec!["search", "report", "searchreport"]);
    }

    #[test]
    fn parses_rg_line() {
        let parsed = parse_match_line("src/auth/session.ts:18:if (redirect) {").unwrap();
        assert_eq!(parsed.path, "src/auth/session.ts");
        assert_eq!(parsed.line_number, 18);
        assert_eq!(parsed.snippet, "if (redirect) {");
    }

    #[test]
    fn coverage_tracks_counts_and_limits() {
        let coverage = SearchCoverage::new(12, 4, MATCH_LIMIT_PER_FILE).finalize(3, 8);
        assert_eq!(coverage.raw_rg_match_count, 12);
        assert_eq!(coverage.raw_candidate_file_count, 4);
        assert_eq!(coverage.displayed_candidate_count, 3);
        assert!(coverage.limited);
        assert_eq!(coverage.match_limit_per_file, MATCH_LIMIT_PER_FILE);
        assert_eq!(coverage.candidate_limit, 8);
        assert!(!coverage.index_used);
        assert_eq!(coverage.index_status, "not_applicable");
    }

    #[test]
    fn coverage_records_index_usage_when_available() {
        let coverage = apply_index_metadata(
            SearchCoverage::new(12, 4, MATCH_LIMIT_PER_FILE),
            true,
            "fresh",
        );

        assert!(coverage.index_used);
        assert_eq!(coverage.index_status, "fresh");
    }
}
