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
    /// Show a compact file card from the index.
    Map {
        /// Path relative to the repo root.
        path: String,
        /// Emit stable JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Locate a symbol in the indexed source files.
    Symbol {
        /// Symbol name to search for.
        name: String,
        /// Emit stable JSON instead of text.
        #[arg(long)]
        json: bool,
    },
}
