mod blast;
mod cli;
mod index;
mod map;
mod output;
mod parser;
mod rank;
mod related;
mod repo;
mod search;
mod semantic;
mod symbol;
mod text;
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
            let filters = rank::FindFilters::try_new(include, exclude, role_filter, match_filter)?;
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

            let next_actions =
                rank::next_actions(&query, &candidates, &repo::display_path(&repo.root));
            let mut coverage = search
                .coverage
                .finalize(candidates.len(), rank::CANDIDATE_LIMIT);
            coverage.limited |= search.match_limit_hit;
            coverage.semantic_status = semantic_status;

            let report = types::FindReport {
                query,
                repo_root: repo::display_path(&repo.root),
                repo_rev: repo.rev,
                latency_ms: started.elapsed().as_millis() as u64,
                coverage,
                candidates,
                next_actions,
            };
            output::write_find_report(&report, json)?;
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
