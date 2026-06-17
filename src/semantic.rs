/// Semantic retrieval foundation for Milestone 8 (see docs/ROADMAP.md).
///
/// This module defines the provider/config boundary, availability check, and error surface.
/// No embedding model is bundled yet. All calls to `require_configured` return an error
/// until a provider is added.
///
/// Future storage paths (not yet created):
///   git repos:     .git/agentgrep/semantic/
///   non-git repos: .agentgrep/semantic/
/// These would hold embedding vectors alongside the existing index.json, never outside
/// the repo boundary, and never in a global or user-level location.
use anyhow::Result;

/// The configured state of the semantic provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SemanticState {
    /// No local embedding provider is configured.
    /// `--semantic` flags on `find` and `index` will return a clear error.
    NotConfigured,
    // Future variant when a provider is bundled:
    // Configured { provider: SemanticProvider, index_path: std::path::PathBuf },
}

/// Returns the current semantic availability state.
/// Always returns `NotConfigured` until a provider is bundled and configured.
pub fn check_availability() -> SemanticState {
    SemanticState::NotConfigured
}

/// Returns `Ok(())` if semantic is available, or an actionable error if not.
/// Call this at the top of any command handler that received `--semantic`.
pub fn require_configured(subcommand: &str) -> Result<()> {
    match check_availability() {
        SemanticState::NotConfigured => Err(anyhow::anyhow!(
            "`agentgrep {subcommand} --semantic` is not yet available: \
no local embedding provider is configured. \
Use `agentgrep {subcommand}` without --semantic for deterministic search. \
See ROADMAP.md Milestone 8 for the planned implementation."
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn availability_is_not_configured() {
        assert_eq!(check_availability(), SemanticState::NotConfigured);
    }

    #[test]
    fn require_configured_returns_error_when_not_configured() {
        let result = require_configured("find \"query\"");
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("--semantic"),
            "error should mention --semantic: {msg}"
        );
        assert!(
            msg.contains("not yet available"),
            "error should say not yet available: {msg}"
        );
    }

    #[test]
    fn require_configured_includes_subcommand_in_error() {
        let result = require_configured("index");
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("agentgrep index --semantic"), "{msg}");
    }
}
