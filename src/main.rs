mod blast;
mod cli;
mod dep_resolve;
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
            let mismatch_note = {
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
                if top_score < 0.30 && !has_strong_signal && !candidates.is_empty() {
                    candidates.truncate(5);
                    Some(
                        "Low-confidence results: query terms may not match codebase vocabulary. \
                         Try rephrasing with identifiers from the codebase, or use `rg` for raw search."
                            .to_string(),
                    )
                } else {
                    None
                }
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
        cli::Commands::Trace { symbol, json } => {
            let repo = repo::discover()?;
            let loaded = index::load(&repo)?;
            let Some(index) = loaded.index.as_ref() else {
                anyhow::bail!(
                    "no index found — run `agentgrep index` first to enable trace"
                );
            };
            let report = trace::build_report(&symbol, index, &repo.root)?;
            trace::write_report(&report, json)?;
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
