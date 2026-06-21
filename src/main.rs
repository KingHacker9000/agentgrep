mod blast;
mod caller_body;
mod cli;
mod dep_resolve;
mod overview;
mod files;
mod index;
mod map;
mod output;
mod parser;
mod peek;
mod rank;
mod related;
mod repo;
mod search;
mod semantic;
mod symbol;
mod text;
mod trace;
mod types;

use anyhow::Result;
use clap::{CommandFactory, Parser};

fn main() {
    if let Err(err) = run() {
        eprintln!("Error: {err}");
        std::process::exit(1);
    }
}

/// Split a camelCase or snake_case identifier into lowercase words.
fn split_identifier(s: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    for c in s.chars() {
        if c == '_' || c == '-' {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
        } else if c.is_uppercase() && !current.is_empty() {
            words.push(std::mem::take(&mut current));
            current.push(c.to_ascii_lowercase());
        } else {
            current.push(c.to_ascii_lowercase());
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

/// Score a vocabulary term against a query by word-level overlap.
/// Returns a value in [0, 1]: fraction of query words that prefix-match any term word.
fn vocab_overlap(query: &str, term: &str) -> f64 {
    let query_words: Vec<String> = query
        .split_whitespace()
        .map(|w| w.to_ascii_lowercase())
        .collect();
    let term_words = split_identifier(term);
    if query_words.is_empty() {
        return 0.0;
    }
    let matched = query_words.iter().filter(|qw| {
        term_words
            .iter()
            .any(|tw| tw.starts_with(qw.as_str()) || qw.starts_with(tw.as_str()))
    });
    matched.count() as f64 / query_words.len() as f64
}

/// Given the original query and a vocabulary list, return the best matching term
/// (if any word overlaps). Short or generic terms are filtered out.
fn best_vocab_term(query: &str, vocab: &[String]) -> Option<String> {
    const GENERIC: &[&str] = &[
        "main", "new", "init", "error", "result", "ok", "err", "none", "some", "config",
        "get", "set", "run", "build", "update", "add", "delete", "create", "from", "into",
        "default", "debug", "clone", "drop", "send", "sync",
    ];
    let mut best_score = 0.0_f64;
    let mut best_term: Option<&str> = None;
    for term in vocab {
        if term.len() < 4 {
            continue;
        }
        if GENERIC.contains(&term.to_ascii_lowercase().as_str()) {
            continue;
        }
        let score = vocab_overlap(query, term);
        if score > best_score {
            best_score = score;
            best_term = Some(term.as_str());
        }
    }
    // Require at least one word to match.
    if best_score > 0.0 {
        best_term.map(|s| s.to_string())
    } else {
        None
    }
}

fn run() -> Result<()> {
    let cli = cli::Cli::parse();

    match cli.command {
        cli::Commands::Find {
            query,
            include,
            exclude,
            role,
            match_mode,
            semantic,
            exclude_docs,
            brief,
            json,
        } => {
            let started = std::time::Instant::now();
            let repo = repo::discover()?;
            let loaded = index::load(&repo)?;
            let index_used = loaded.index.is_some();
            let index_status = if index_used {
                loaded.state.to_string()
            } else {
                "not_applicable".to_string()
            };
            let search = search::run_with_index(&repo.root, &query, index_used, &index_status)?;
            let role_filter = match role {
                cli::FindRoleSelection::Source => rank::FindRoleFilter::Source,
                cli::FindRoleSelection::Doc => rank::FindRoleFilter::Doc,
                cli::FindRoleSelection::Config => rank::FindRoleFilter::Config,
                cli::FindRoleSelection::Test => rank::FindRoleFilter::Test,
                cli::FindRoleSelection::Other => rank::FindRoleFilter::Other,
                cli::FindRoleSelection::Any => rank::FindRoleFilter::Any,
            };
            let match_filter = match match_mode {
                cli::FindMatchSelection::Any => rank::FindMatchFilter::Any,
                cli::FindMatchSelection::All => rank::FindMatchFilter::All,
            };
            let mut filters =
                rank::FindFilters::try_new(include, exclude, role_filter, match_filter)?;
            filters.exclude_docs = exclude_docs;
            let det_candidates = rank::rank_with_index(
                &query,
                search.matches,
                loaded.index.as_ref(),
                &index_status,
                &filters,
            );

            // Semantic expansion: merge candidates + label evidence.
            let (candidates, semantic_status) = if semantic {
                semantic::expand_candidates(&repo, &query, det_candidates)?
            } else {
                (det_candidates, "not_requested".to_string())
            };

            let mut candidates = candidates;

            // Build vocabulary from symbols before tiered density strips them.
            let vocabulary = rank::build_vocabulary(&candidates, 12);

            rank::apply_tiered_density(&mut candidates);

            let next_actions =
                rank::next_actions(&query, &candidates, &repo::display_path(&repo.root));
            let mut coverage = search
                .coverage
                .finalize(candidates.len(), rank::CANDIDATE_LIMIT);
            coverage.limited |= search.match_limit_hit;
            coverage.semantic_status = semantic_status;

            // Vocabulary mismatch: top score below 0.30 and no strong evidence found.
            let top_score = candidates.first().map(|c| c.score).unwrap_or(0.0);
            let has_strong_signal = candidates.iter().any(|c| {
                c.evidence.iter().any(|e| {
                    matches!(
                        e.evidence_type.as_str(),
                        "exact_phrase_match"
                            | "near_phrase_match"
                            | "indexed_symbol_definition"
                            | "indexed_symbol_reference"
                    )
                })
            });
            // Mismatch: no strong signal AND the best result is still low-confidence.
            // Graph-edge boosts can inflate scores to ~0.37 even when no query terms exist in
            // the codebase, so we check the top candidate's Confidence in addition to the score.
            let top_conf_low = candidates
                .first()
                .map(|c| c.confidence == types::Confidence::Low)
                .unwrap_or(false);
            let is_mismatch = !has_strong_signal
                && !candidates.is_empty()
                && (top_score < 0.30 || (top_conf_low && top_score < 0.40));

            let (mismatch_note, auto_expansion) = if is_mismatch {
                // Build a wider vocab and try to find the best matching term.
                let wide_vocab = rank::build_vocabulary(&candidates, 20);
                let best_term = best_vocab_term(&query, &wide_vocab);

                let expansion = best_term.and_then(|requery| {
                    let rs = search::run_with_index(
                        &repo.root,
                        &requery,
                        index_used,
                        &index_status,
                    )
                    .ok()?;
                    let mut rc = rank::rank_with_index(
                        &requery,
                        rs.matches,
                        loaded.index.as_ref(),
                        &index_status,
                        &filters,
                    );
                    rank::apply_tiered_density(&mut rc);
                    rc.truncate(5);
                    if rc.is_empty() {
                        return None;
                    }
                    Some(types::AutoExpansion {
                        original_query: query.clone(),
                        requery,
                        candidates: rc,
                    })
                });

                let note = match &expansion {
                    Some(exp) => format!(
                        "Low confidence for \"{}\" — auto-expanded to \"{}\"",
                        query, exp.requery
                    ),
                    None => "Low-confidence results: query terms may not match codebase \
                             vocabulary. Try rephrasing with identifiers from the codebase, \
                             or use `rg` for raw search."
                        .to_string(),
                };
                candidates.truncate(5);
                (Some(note), expansion)
            } else {
                (None, None)
            };

            let report = types::FindReport {
                query,
                repo_root: repo::display_path(&repo.root),
                repo_rev: repo.rev,
                latency_ms: started.elapsed().as_millis() as u64,
                coverage,
                candidates,
                next_actions,
                note: mismatch_note,
                vocabulary,
                auto_expansion,
            };
            output::write_find_report(&report, json, brief)?;
        }
        cli::Commands::Index {
            status,
            clear,
            semantic,
            yes,
        } => {
            let repo = repo::discover()?;
            if clear {
                let report = index::clear(&repo)?;
                index::write_clear_report(&report)?;
            } else if status {
                let report = index::status(&repo)?;
                index::write_status_report(&report)?;
            } else {
                let report = index::build(&repo)?;
                index::write_build_report(&report)?;
                if semantic {
                    let loaded = index::load(&repo)?;
                    let sem_report = semantic::build_semantic(&repo, loaded.index.as_ref(), yes)?;
                    semantic::write_semantic_build_report(&sem_report);
                }
            }
        }
        cli::Commands::Map { path, json } => {
            let repo = repo::discover()?;
            let report = map::build_report(&repo, &path)?;
            map::write_report(&report, json)?;
        }
        cli::Commands::Symbol { name, json } => {
            let repo = repo::discover()?;
            let report = symbol::build_report(&repo, &name)?;
            symbol::write_report(&report, json)?;
        }
        cli::Commands::Related { query, json } => {
            let repo = repo::discover()?;
            let report = related::build_report(&repo, &query)?;
            related::write_report(&report, json)?;
        }
        cli::Commands::Blast { query, json } => {
            let repo = repo::discover()?;
            let report = blast::build_report(&repo, &query)?;
            blast::write_report(&report, json)?;
        }
        cli::Commands::Peek {
            symbol,
            file,
            line,
            context,
            json,
        } => {
            let repo = repo::discover()?;
            let loaded = index::load(&repo)?;
            let Some(index) = loaded.index.as_ref() else {
                anyhow::bail!(
                    "no index found — run `agentgrep index` first to enable peek"
                );
            };
            let report = peek::peek_symbol(
                &symbol,
                file.as_deref(),
                line,
                context,
                index,
                &repo::display_path(&repo.root),
            )?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!(
                    "{}  {}:{}-{}",
                    report.kind, report.file_path, report.line_number, report.end_line
                );
                if let Some(sig) = &report.signature {
                    println!("{sig}");
                }
                println!();
                for ln in &report.body {
                    println!("{:4}  {}", ln.line, ln.text);
                }
            }
        }
        cli::Commands::Files { pattern, json } => {
            let repo = repo::discover()?;
            let loaded = index::load(&repo)?;
            let Some(index) = loaded.index.as_ref() else {
                anyhow::bail!(
                    "no index found — run `agentgrep index` first to enable files"
                );
            };
            let report = files::build_report(&repo, &pattern, index)?;
            files::write_report(&report, json)?;
        }
        cli::Commands::Trace {
            symbol,
            callers_body,
            include_tests,
            json,
        } => {
            let repo = repo::discover()?;
            let loaded = index::load(&repo)?;
            let Some(index) = loaded.index.as_ref() else {
                anyhow::bail!(
                    "no index found — run `agentgrep index` first to enable trace"
                );
            };
            let report =
                trace::build_report(&symbol, index, &repo.root, callers_body, include_tests)?;
            trace::write_report(&report, json)?;
        }
        cli::Commands::Overview { full, min_refs, only, json } => {
            let repo = repo::discover()?;
            let loaded = index::load(&repo)?;
            let Some(index) = loaded.index.as_ref() else {
                anyhow::bail!(
                    "no index found — run `agentgrep index` first to enable overview"
                );
            };
            let report = overview::build_report(index, full, min_refs, &only)?;
            overview::write_report(&report, json)?;
        }
        cli::Commands::Semantic { action } => {
            let repo = repo::discover()?;
            match action {
                cli::SemanticAction::Status => {
                    let report = semantic::semantic_status(&repo)?;
                    semantic::write_semantic_status_report(&report);
                }
                cli::SemanticAction::Clean {
                    repo_index,
                    model,
                    all,
                } => {
                    let report = if all {
                        semantic::clean_all(&repo)?
                    } else if repo_index {
                        semantic::clean_repo_index(&repo)?
                    } else if model {
                        semantic::clean_model_cache()?
                    } else {
                        anyhow::bail!(
                            "specify at least one of --repo-index, --model, or --all.\n\
                             Run `agentgrep semantic clean --help` for options."
                        )
                    };
                    semantic::write_semantic_clean_report(&report);
                }
            }
        }
        cli::Commands::Completions { shell } => {
            let mut cmd = cli::Cli::command();
            clap_complete::generate(shell, &mut cmd, "agentgrep", &mut std::io::stdout());
        }
    }

    Ok(())
}
