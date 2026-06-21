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
        /// Enable semantic candidate expansion and reranking (requires a configured local provider).
        #[arg(
            long,
            help = "Enable semantic candidate expansion and reranking (requires configured provider)."
        )]
        semantic: bool,
        /// Hard-exclude doc, lockfile, and generated files from results. Keeps output focused on source code.
        #[arg(long, help = "Exclude doc, lockfile, and generated files from results.")]
        exclude_docs: bool,
        /// Compact one-line-per-candidate output. Useful for agent pipelines to minimize context.
        #[arg(long, help = "Compact one-line-per-candidate output.")]
        brief: bool,
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
        /// Prepare semantic embedding data in addition to the standard index.
        #[arg(
            long,
            help = "Prepare semantic embedding data alongside the standard index."
        )]
        semantic: bool,
        /// Automatically accept the embedding model download prompt (for scripts and CI).
        #[arg(long, help = "Automatically accept download prompts.")]
        yes: bool,
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
    /// Show the body of an indexed symbol (requires index).
    Peek {
        /// Symbol name to peek at.
        symbol: String,
        /// File path to disambiguate when the symbol appears in multiple files.
        #[arg(long, value_name = "FILE")]
        file: Option<String>,
        /// Line number to disambiguate when the same symbol appears multiple times in one file.
        #[arg(long, value_name = "LINE")]
        line: Option<usize>,
        /// Extra context lines to show before and after the symbol body.
        #[arg(long, value_name = "N", default_value_t = 0)]
        context: usize,
        /// Write stable JSON instead of text.
        #[arg(long, help = "Write stable JSON instead of text.")]
        json: bool,
    },
    /// List indexed files matching a name pattern (requires index).
    Files {
        /// Substring or glob pattern to match against file paths.
        pattern: String,
        /// Write stable JSON instead of text.
        #[arg(long, help = "Write stable JSON instead of text.")]
        json: bool,
    },
    /// Show callers and callees for an indexed symbol (requires index).
    Trace {
        /// Symbol name to trace.
        symbol: String,
        /// Write stable JSON instead of text.
        #[arg(long, help = "Write stable JSON instead of text.")]
        json: bool,
    },
    /// Inspect or clean the semantic index and model cache.
    Semantic {
        #[command(subcommand)]
        action: SemanticAction,
    },
    /// Print shell completions to stdout and exit.
    #[command(hide = true)]
    Completions {
        /// Shell to generate completions for.
        shell: Shell,
    },
}

#[derive(Debug, Subcommand)]
pub enum SemanticAction {
    /// Show the state of the repo semantic index and global model cache.
    Status,
    /// Remove semantic data (semantic index and/or model cache).
    Clean {
        /// Remove the repo-local semantic index (meta.json + vectors.bin).
        #[arg(long, conflicts_with_all = ["model", "all"])]
        repo_index: bool,
        /// Remove the global model cache directory.
        #[arg(long, conflicts_with_all = ["repo_index", "all"])]
        model: bool,
        /// Remove both the repo-local semantic index and the model cache.
        #[arg(long, conflicts_with_all = ["repo_index", "model"])]
        all: bool,
    },
}
