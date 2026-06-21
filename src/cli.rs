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
    about = "Evidence-first codebase search and navigation for coding agents",
    long_about = "Agentgrep uses rg as its recall floor and a lightweight local index for symbol, graph, and vocabulary context. No LLM, daemon, watcher, or background service required.\n\nAll commands accept --json for stable, parseable output. Use --json in any automated or agent context.",
    after_help = "Session start : overview (orientation) → find --brief (locate files)\nNavigation   : trace (call graph + dep status) → peek (read body) → files (confirm paths)\nBefore edit  : related (neighbors) → blast (impact radius)"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Ranked file search. Call first for any task. Use --brief for compact output with a vocabulary
    /// line; if results are weak (score < 0.30), use vocabulary terms to requery with
    /// codebase-native identifiers. Use --json for all automated or agent contexts.
    Find {
        /// Natural language or identifier query. Tip: use symbol names from `overview` vocab for best results.
        query: String,
        /// Repeatable path glob to include. Bare globs like `*.css` match by basename anywhere.
        #[arg(long = "include", value_name = "GLOB")]
        include: Vec<String>,
        /// Repeatable path glob to exclude. Bare globs like `*.css` match by basename anywhere.
        #[arg(long = "exclude", value_name = "GLOB")]
        exclude: Vec<String>,
        /// Restrict results to a specific file role: source, doc, config, test, other.
        #[arg(long, value_enum, default_value_t = FindRoleSelection::Any)]
        role: FindRoleSelection,
        /// Require files to match any (default) or all significant query terms.
        #[arg(long = "match", value_enum, default_value_t = FindMatchSelection::Any)]
        match_mode: FindMatchSelection,
        /// Enable semantic candidate expansion and reranking (requires configured local provider).
        #[arg(long, help = "Enable semantic candidate expansion and reranking (requires configured provider).")]
        semantic: bool,
        /// Exclude doc, lockfile, and generated files. Keeps results focused on source code.
        #[arg(long, help = "Exclude doc, lockfile, and generated files from results.")]
        exclude_docs: bool,
        /// One-line-per-candidate output ending with a vocab: line of symbol names from top results.
        /// Recommended for agent use — minimizes context while surfacing vocabulary for follow-up queries.
        #[arg(long, help = "Compact one-line-per-candidate output with vocabulary line.")]
        brief: bool,
        /// Write stable JSON instead of text.
        #[arg(long, help = "Write stable JSON instead of text.")]
        json: bool,
    },
    /// Build or refresh the local repository index. Required before trace, peek, files, overview,
    /// and map/related/blast with full graph context. Run once per session; use --status to check
    /// freshness before re-running.
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
    /// Full file inspection: role, defined symbols, incoming callers, outgoing dependencies.
    /// Use after find returns a candidate to understand its place in the codebase without reading
    /// the whole file. incoming_edges = who imports/calls this; outgoing_edges = what it depends on.
    Map {
        /// Path relative to the repo root.
        path: String,
        /// Write stable JSON instead of text.
        #[arg(long, help = "Write stable JSON instead of text.")]
        json: bool,
    },
    /// Definitions and reference sites for a symbol name. Tries exact match, then
    /// case-insensitive, then substring. Check used_by context to distinguish production
    /// references from test/fixture-only references. Prefer `trace` for call graph detail.
    Symbol {
        /// Symbol name to search for.
        name: String,
        /// Write stable JSON instead of text.
        #[arg(long, help = "Write stable JSON instead of text.")]
        json: bool,
    },
    /// Files connected to a path or symbol by imports, symbol references, or shared edges.
    /// Use before editing to understand the neighborhood. High-confidence results share
    /// explicit import/reference edges; same_area results share only directory proximity.
    Related {
        /// File path or symbol query to analyze.
        query: String,
        /// Write stable JSON instead of text.
        #[arg(long, help = "Write stable JSON instead of text.")]
        json: bool,
    },
    /// Conservative impact estimate: what else might break if this file or symbol changes.
    /// Call before any non-trivial edit. risk_level (low/medium/high) guides inspection depth;
    /// suggested_inspection_order lists files to check. Not exhaustive — files not listed may
    /// still be affected through dynamic dispatch or runtime paths.
    Blast {
        /// File path or symbol query to analyze.
        query: String,
        /// Write stable JSON instead of text.
        #[arg(long, help = "Write stable JSON instead of text.")]
        json: bool,
    },
    /// Read a symbol's implementation body from the index without opening the file.
    /// Use after find or trace identifies a symbol. When trace returns multiple defined_in
    /// entries, pass --file to select the right one. --context N adds N surrounding lines.
    Peek {
        /// Symbol name to read.
        symbol: String,
        /// File path to select when the symbol is defined in multiple files.
        #[arg(long, value_name = "FILE")]
        file: Option<String>,
        /// Line number to select when the same symbol appears multiple times in one file.
        #[arg(long, value_name = "LINE")]
        line: Option<usize>,
        /// Lines of surrounding context to include before and after the symbol body.
        #[arg(long, value_name = "N", default_value_t = 0)]
        context: usize,
        /// Write stable JSON instead of text.
        #[arg(long, help = "Write stable JSON instead of text.")]
        json: bool,
    },
    /// List indexed files whose paths match a pattern. Use to confirm exact file paths before
    /// opening, or to check whether a file is tracked by the index. Supports substring and
    /// glob patterns against the full relative path.
    Files {
        /// Substring or glob to match against indexed file paths (e.g. "auth", "src/*.rs").
        pattern: String,
        /// Write stable JSON instead of text.
        #[arg(long, help = "Write stable JSON instead of text.")]
        json: bool,
    },
    /// Call graph for a symbol: definitions, callers, callees, and external-dep resolution.
    /// index_status: "found" (defined here — peek the body) | "external" (from a dep —
    /// dep_package names it; callers show usage) | "not_found" (follow next_actions[0]).
    /// Empty callers[] does not mean unused — only indexed references are captured.
    Trace {
        /// Symbol name to trace (exact or case-insensitive match).
        symbol: String,
        /// For each caller, include the AST-extracted containing function body.
        /// Returns the full function enclosing the call site (capped at 60 lines with
        /// smart truncation keeping the call site always visible). Capped at 10 callers.
        #[arg(long)]
        callers_body: bool,
        /// Separate test-file callers into test_callers[] instead of mixing with production
        /// callers. With --callers-body, test caller bodies are also included (max 5).
        #[arg(long)]
        include_tests: bool,
        /// Write stable JSON instead of text.
        #[arg(long, help = "Write stable JSON instead of text.")]
        json: bool,
    },
    /// Cold-start codebase orientation. Run once per session before your first find call.
    /// Returns entry points, package/crate structure, public types ranked by reference count,
    /// and a vocabulary line of key symbol names. Use vocabulary to anchor find queries with
    /// codebase-native identifiers instead of guessing generic terms.
    ///
    /// Sections: types, functions, packages, entries, connected, vocab.
    /// Default shows all sections with top 20 types. --full removes the cap.
    /// --only vocab is the lightest call (~50 bytes) for pure vocabulary priming.
    Overview {
        /// Show all public types AND all public functions, uncapped.
        #[arg(long)]
        full: bool,
        /// Exclude symbols with fewer than N references. Filters noise on large codebases.
        /// Suggested values: --min-refs 2 (some signal), --min-refs 5 (core API only).
        #[arg(long, default_value_t = 0, value_name = "N")]
        min_refs: usize,
        /// Show only the listed sections. Comma-separated: types,functions,packages,entries,connected,vocab.
        /// Examples: --only vocab  |  --only types,functions  |  --only packages,entries
        #[arg(long, value_delimiter = ',', value_name = "SECTIONS")]
        only: Vec<String>,
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
