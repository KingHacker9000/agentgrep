use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "agentgrep", version, about = "Evidence-first codebase search")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Rank likely files for a query.
    Find {
        /// Query to search for.
        query: String,
        /// Emit stable JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Build or inspect the lightweight repository index.
    Index {
        /// Show index status instead of rebuilding.
        #[arg(long, conflicts_with = "clear")]
        status: bool,
        /// Clear the stored index.
        #[arg(long, conflicts_with = "status")]
        clear: bool,
    },
}
