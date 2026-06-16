use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "agentgrep",
    version,
    about = "Evidence-first codebase search and navigation",
    long_about = "Agentgrep uses rg as its recall floor. The index is optional and improves ranking and context when present. No LLM, daemon, watcher, or background service is required.",
    after_help = "Workflow:\n  find -> index -> map -> symbol -> related -> blast"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Evidence-first search for likely files.
    Find {
        /// Query to search for.
        query: String,
        /// Write stable JSON instead of text.
        #[arg(long, help = "Write stable JSON instead of text.")]
        json: bool,
    },
    /// Build or inspect the lightweight repository index.
    Index {
        /// Show index status instead of rebuilding.
        #[arg(
            long,
            conflicts_with = "clear",
            help = "Show index status instead of rebuilding."
        )]
        status: bool,
        /// Clear the stored index.
        #[arg(long, conflicts_with = "status", help = "Clear the stored index.")]
        clear: bool,
    },
    /// Inspect one file with indexed context.
    Map {
        /// Path relative to the repo root.
        path: String,
        /// Write stable JSON instead of text.
        #[arg(long, help = "Write stable JSON instead of text.")]
        json: bool,
    },
    /// Find definitions and references for a symbol.
    Symbol {
        /// Symbol name to search for.
        name: String,
        /// Write stable JSON instead of text.
        #[arg(long, help = "Write stable JSON instead of text.")]
        json: bool,
    },
    /// Inspect nearby files, symbols, and references.
    Related {
        /// File path or symbol query to analyze.
        query: String,
        /// Write stable JSON instead of text.
        #[arg(long, help = "Write stable JSON instead of text.")]
        json: bool,
    },
    /// Estimate conservative impact for a file or symbol.
    Blast {
        /// File path or symbol query to analyze.
        query: String,
        /// Write stable JSON instead of text.
        #[arg(long, help = "Write stable JSON instead of text.")]
        json: bool,
    },
}
