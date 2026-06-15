mod cli;
mod index;
mod map;
mod output;
mod rank;
mod related;
mod repo;
mod search;
mod symbol;
mod text;
mod types;

use anyhow::Result;
use clap::Parser;

fn main() {
    if let Err(err) = run() {
        eprintln!("Error: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = cli::Cli::parse();

    match cli.command {
        cli::Commands::Find { query, json } => {
            let started = std::time::Instant::now();
            let repo = repo::discover()?;
            let search = search::run(&repo.root, &query)?;
            let candidates = rank::rank(&query, search.matches);
            let next_actions =
                rank::next_actions(&query, &candidates, &repo::display_path(&repo.root));
            let mut coverage = search
                .coverage
                .finalize(candidates.len(), rank::CANDIDATE_LIMIT);
            coverage.limited |= search.match_limit_hit;
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
        cli::Commands::Index { status, clear } => {
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
    }

    Ok(())
}
