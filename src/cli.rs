use clap::{Parser, Subcommand, ValueEnum};
use clap_complete::Shell;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum FindRoleSelection {
    Source,
    Doc,
    Config,
    Test,
    Other,
    Any,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum FindMatchSelection {
    Any,
    All,
}

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
        /// Repeatable path glob to include. Bare globs like `*.css` match by basename anywhere.
        #[arg(long = "include", value_name = "GLOB")]
        include: Vec<String>,
        /// Repeatable path glob to exclude. Bare globs like `*.css` match by basename anywhere.
        #[arg(long = "exclude", value_name = "GLOB")]
        exclude: Vec<String>,
        /// Prefer a specific file role.
        #[arg(long, value_enum, default_value_t = FindRoleSelection::Any)]
        role: FindRoleSelection,
        /// Control whether files must match any or all significant query terms.
        #[arg(long = "match", value_enum, default_value_t = FindMatchSelection::Any)]
        match_mode: FindMatchSelection,
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
    /// Print shell completions to stdout and exit.
    #[command(hide = true)]
    Completions {
        /// Shell to generate completions for.
        shell: Shell,
    },
}
