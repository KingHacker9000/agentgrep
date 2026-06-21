use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize)]
pub struct FindReport {
    pub query: String,
    pub repo_root: String,
    pub repo_rev: Option<String>,
    pub latency_ms: u64,
    pub coverage: SearchCoverage,
    pub candidates: Vec<FileCandidate>,
    pub next_actions: Vec<String>,
    /// Present when results are low-confidence and the query may need reformulation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Codebase vocabulary: top matching symbol names surfaced from the index.
    /// Helps agents learn the exact identifiers used in this repo for the query concept.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vocabulary: Vec<String>,
}

/// A compact symbol reference attached to a find candidate.
#[derive(Debug, Clone, Serialize)]
pub struct SymbolSummary {
    pub name: String,
    pub kind: String,
    pub line: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_class: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileCandidate {
    pub path: String,
    pub kind: String,
    pub role: String,
    pub score: f64,
    pub confidence: Confidence,
    pub detail_level: DetailLevel,
    pub line_ranges: Vec<LineRange>,
    pub snippets: Vec<Snippet>,
    pub evidence: Vec<Evidence>,
    /// Index-matched symbols in this file relevant to the query.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub symbols: Vec<SymbolSummary>,
}

/// Controls how much per-candidate data is included in the output.
/// Assigned in `rank_with_index` based on score thresholds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DetailLevel {
    /// score >= 0.70 — all fields present
    Full,
    /// 0.45 <= score < 0.70 — snippets present, evidence trimmed to 2
    Medium,
    /// 0.25 <= score < 0.45 — snippets dropped, 1 evidence entry
    Minimal,
    /// score < 0.25 — path/role/score/confidence only
    Enum,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    High,
    Medium,
    Low,
}

impl std::fmt::Display for Confidence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Confidence::High => "high",
            Confidence::Medium => "medium",
            Confidence::Low => "low",
        };
        write!(f, "{value}")
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LineRange {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Snippet {
    pub line_number: usize,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Evidence {
    #[serde(rename = "type")]
    pub evidence_type: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchCoverage {
    pub raw_rg_match_count: usize,
    pub raw_candidate_file_count: usize,
    pub displayed_candidate_count: usize,
    pub limited: bool,
    pub match_limit_per_file: usize,
    pub candidate_limit: usize,
    pub index_used: bool,
    pub index_status: String,
    /// Whether semantic retrieval contributed to this response.
    /// "not_requested": --semantic was not passed (default).
    /// Future value "active" when a provider is configured and used.
    pub semantic_status: String,
}

impl SearchCoverage {
    pub fn new(
        raw_rg_match_count: usize,
        raw_candidate_file_count: usize,
        match_limit_per_file: usize,
    ) -> Self {
        Self {
            raw_rg_match_count,
            raw_candidate_file_count,
            displayed_candidate_count: 0,
            limited: false,
            match_limit_per_file,
            candidate_limit: 0,
            index_used: false,
            index_status: "not_applicable".to_string(),
            semantic_status: "not_requested".to_string(),
        }
    }

    pub fn finalize(mut self, displayed_candidate_count: usize, candidate_limit: usize) -> Self {
        self.displayed_candidate_count = displayed_candidate_count;
        self.candidate_limit = candidate_limit;
        self.limited = displayed_candidate_count < self.raw_candidate_file_count;
        self
    }
}

/// Report for `agentgrep files <pattern>` — confirmed file paths from the index.
#[derive(Debug, Clone, Serialize)]
pub struct FilesReport {
    pub pattern: String,
    pub repo_root: String,
    pub total_indexed: usize,
    pub matches: Vec<FileMatch>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileMatch {
    pub path: String,
    pub role: String,
}

/// A single external-dependency import record captured at index time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepImport {
    /// The imported name (leaf symbol or module alias), e.g. "SyntaxSet", "Blueprint".
    pub symbol_or_module: String,
    /// The top-level package/crate that provides it, e.g. "syntect", "flask".
    pub dep_package: String,
    pub file_path: String,
    pub line: usize,
}

/// Report for `agentgrep trace <symbol>` — caller/callee graph.
#[derive(Debug, Clone, Serialize)]
pub struct TraceReport {
    pub symbol: String,
    /// "found" | "external" | "not_found"
    pub index_status: String,
    pub defined_in: Vec<TraceDefinition>,
    pub callers: Vec<TraceCallSite>,
    pub callees: Vec<TraceCallSite>,
    pub next_actions: Vec<String>,
    /// Set when the symbol is determined to be from an external dependency.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dep_package: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TraceDefinition {
    pub file: String,
    pub line: usize,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_class: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TraceCallSite {
    pub file: String,
    pub line: usize,
    pub context: String,
    pub confidence: String,
}

/// Report for `agentgrep overview` — lightweight codebase orientation.
#[derive(Debug, Clone, Serialize)]
pub struct OverviewReport {
    pub repo_root: String,
    pub languages: Vec<String>,
    pub file_count: usize,
    pub symbol_count: usize,
    pub entry_points: Vec<String>,
    /// Top-level source directories grouped by path prefix, with file counts.
    pub packages: Vec<PackageGroup>,
    /// Public structs / enums / traits / classes / interfaces (always shown).
    pub key_types: Vec<OverviewSymbol>,
    /// Public functions / methods (shown only with --full).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub key_functions: Vec<OverviewSymbol>,
    /// Most heavily connected file pairs.
    pub most_connected: Vec<ConnectedPair>,
    /// Top symbol names for vocabulary priming.
    pub vocabulary: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OverviewSymbol {
    pub name: String,
    pub kind: String,
    pub file: String,
    pub line: usize,
    pub ref_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct PackageGroup {
    pub prefix: String,
    pub source_file_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConnectedPair {
    pub from: String,
    pub to: String,
    pub edge_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct MapReport {
    pub path: String,
    pub role: String,
    pub index_status: String,
    pub index_path: String,
    pub repo_rev: Option<String>,
    pub size_bytes: Option<u64>,
    pub modified_unix: Option<u64>,
    pub content_hash: Option<String>,
    pub symbols: Vec<IndexedSymbol>,
    pub outgoing_edges: Vec<MapEdge>,
    pub incoming_edges: Vec<MapEdge>,
    pub connection_counts: ConnectionCounts,
    pub next_actions: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SymbolReport {
    pub query: String,
    pub index_status: String,
    pub match_mode: SymbolMatchMode,
    pub matches: Vec<SymbolMatch>,
    pub next_actions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RelatedMode {
    File,
    Symbol,
}

impl std::fmt::Display for RelatedMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            RelatedMode::File => "file",
            RelatedMode::Symbol => "symbol",
        };
        write!(f, "{value}")
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RelatedFile {
    pub path: String,
    pub role: String,
    pub score: f64,
    pub confidence: Confidence,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RelatedReport {
    pub query: String,
    pub mode: RelatedMode,
    pub index_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub match_mode: Option<SymbolMatchMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_role: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub symbol_matches: Vec<SymbolMatch>,
    pub related_files: Vec<RelatedFile>,
    pub edges: Vec<MapEdge>,
    pub symbols: Vec<IndexedSymbol>,
    pub references: Vec<crate::index::IndexedSymbolReference>,
    pub next_actions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BlastMode {
    File,
    Symbol,
}

impl std::fmt::Display for BlastMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            BlastMode::File => "file",
            BlastMode::Symbol => "symbol",
        };
        write!(f, "{value}")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BlastRiskLevel {
    Low,
    Medium,
    High,
}

impl std::fmt::Display for BlastRiskLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            BlastRiskLevel::Low => "low",
            BlastRiskLevel::Medium => "medium",
            BlastRiskLevel::High => "high",
        };
        write!(f, "{value}")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BlastImpactContext {
    Production,
    TestFixture,
    SameArea,
    Unknown,
}

impl std::fmt::Display for BlastImpactContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            BlastImpactContext::Production => "production",
            BlastImpactContext::TestFixture => "test/fixture",
            BlastImpactContext::SameArea => "same_area",
            BlastImpactContext::Unknown => "unknown",
        };
        write!(f, "{value}")
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct BlastImpactedFile {
    pub path: String,
    pub role: String,
    pub score: f64,
    pub confidence: Confidence,
    pub context: BlastImpactContext,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BlastReport {
    pub query: String,
    pub mode: BlastMode,
    pub index_status: String,
    pub risk_level: BlastRiskLevel,
    pub risk_reasons: Vec<String>,
    pub impacted_files: Vec<BlastImpactedFile>,
    pub affected_symbols: Vec<IndexedSymbol>,
    pub references: Vec<crate::index::IndexedSymbolReference>,
    pub suggested_inspection_order: Vec<String>,
    pub next_actions: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SymbolMatch {
    pub symbol: IndexedSymbol,
    pub file_role: String,
    pub used_by: Vec<crate::index::IndexedSymbolReference>,
    pub outgoing_edges: Vec<MapEdge>,
    pub incoming_edges: Vec<MapEdge>,
    pub next_actions: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MapEdge {
    pub edge_type: String,
    pub from: String,
    pub to: String,
    pub confidence: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConnectionCounts {
    pub outgoing_total: usize,
    pub incoming_total: usize,
    pub outgoing_by_type: BTreeMap<String, usize>,
    pub incoming_by_type: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SymbolMatchMode {
    Exact,
    CaseInsensitive,
    Substring,
    None,
}

impl std::fmt::Display for SymbolMatchMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            SymbolMatchMode::Exact => "exact",
            SymbolMatchMode::CaseInsensitive => "case_insensitive",
            SymbolMatchMode::Substring => "substring",
            SymbolMatchMode::None => "none",
        };
        write!(f, "{value}")
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "lowercase")]
pub enum SymbolKind {
    #[default]
    Function,
    Struct,
    Enum,
    Trait,
    Impl,
    TypeAlias,
    Const,
    Static,
    Module,
    Unknown,
}

impl std::fmt::Display for SymbolKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            SymbolKind::Function => "function",
            SymbolKind::Struct => "struct",
            SymbolKind::Enum => "enum",
            SymbolKind::Trait => "trait",
            SymbolKind::Impl => "impl",
            SymbolKind::TypeAlias => "type_alias",
            SymbolKind::Const => "const",
            SymbolKind::Static => "static",
            SymbolKind::Module => "module",
            SymbolKind::Unknown => "unknown",
        };
        write!(f, "{value}")
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Visibility {
    #[default]
    Public,
    Private,
}

impl std::fmt::Display for Visibility {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Visibility::Public => "public",
            Visibility::Private => "private",
        };
        write!(f, "{value}")
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IndexedSymbol {
    pub name: String,
    pub kind: SymbolKind,
    pub file_path: String,
    pub line_number: usize,
    pub visibility: Visibility,
    pub signature: Option<String>,
    /// Last line of the symbol's body (from tree-sitter extent). None for old indexes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_line: Option<usize>,
    /// For class methods: the name of the enclosing class. Enables `symbol "ClassName.method"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_class: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SearchMatch {
    pub path: String,
    pub line_number: usize,
    pub snippet: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::{EdgeConfidence, ReferenceContext};
    use serde_json::Value;

    fn sample_indexed_symbol(name: &str, file_path: &str) -> IndexedSymbol {
        IndexedSymbol {
            name: name.to_string(),
            kind: SymbolKind::Function,
            file_path: file_path.to_string(),
            line_number: 12,
            visibility: Visibility::Public,
            signature: Some(format!("pub fn {name}()")),
            end_line: None,

            parent_class: None,        }
    }

    fn sample_reference(
        symbol_name: &str,
        from_file: &str,
        target_file: &str,
    ) -> crate::index::IndexedSymbolReference {
        crate::index::IndexedSymbolReference {
            from_file: from_file.to_string(),
            symbol_name: symbol_name.to_string(),
            target_file: Some(target_file.to_string()),
            target_line: Some(12),
            line_number: 3,
            confidence: EdgeConfidence::Extracted,
            reason: "use statement reference".to_string(),
            context: ReferenceContext::Production,
            additional_count: 0,
        }
    }

    fn sample_edge(edge_type: &str, from: &str, to: &str) -> MapEdge {
        MapEdge {
            edge_type: edge_type.to_string(),
            from: from.to_string(),
            to: to.to_string(),
            confidence: "extracted".to_string(),
            reason: format!("{edge_type} edge"),
        }
    }

    fn assert_top_level_fields(json: &Value, fields: &[&str]) {
        for field in fields {
            assert!(
                json.get(field).is_some(),
                "expected top-level field `{field}` to exist in {json}"
            );
        }
    }

    #[test]
    fn coverage_serializes_with_expected_fields() {
        let coverage = SearchCoverage::new(9, 4, 20).finalize(3, 8);
        let json = serde_json::to_value(&coverage).unwrap();

        assert_eq!(json["raw_rg_match_count"], 9);
        assert_eq!(json["raw_candidate_file_count"], 4);
        assert_eq!(json["displayed_candidate_count"], 3);
        assert_eq!(json["limited"], true);
        assert_eq!(json["match_limit_per_file"], 20);
        assert_eq!(json["candidate_limit"], 8);
        assert_eq!(json["index_used"], false);
        assert_eq!(json["index_status"], "not_applicable");
        assert_eq!(json["semantic_status"], "not_requested");
    }

    #[test]
    fn coverage_semantic_status_defaults_to_not_requested() {
        let coverage = SearchCoverage::new(0, 0, 20).finalize(0, 8);
        let json = serde_json::to_value(&coverage).unwrap();
        assert_eq!(
            json["semantic_status"], "not_requested",
            "semantic_status must default to not_requested when --semantic is not passed"
        );
    }

    #[test]
    fn find_report_serializes_stable_top_level_fields() {
        let report = FindReport {
            query: "SearchResult".to_string(),
            repo_root: "C:/repo".to_string(),
            repo_rev: Some("abc".to_string()),
            latency_ms: 42,
            coverage: SearchCoverage::new(5, 2, 20).finalize(2, 8),
            candidates: vec![FileCandidate {
                path: "src/search.rs".to_string(),
                kind: "file".to_string(),
                role: "source".to_string(),
                score: 0.9,
                confidence: Confidence::High,
                detail_level: DetailLevel::Full,
                line_ranges: vec![LineRange { start: 12, end: 12 }],
                snippets: vec![Snippet {
                    line_number: 12,
                    text: "pub struct SearchResult".to_string(),
                }],
                evidence: vec![Evidence {
                    evidence_type: "rg_match".to_string(),
                    detail: "matched SearchResult".to_string(),
                }],
                symbols: vec![],
            }],
            next_actions: vec!["agentgrep map src/search.rs".to_string()],
            note: None,
            vocabulary: vec![],
        };

        let json = serde_json::to_value(&report).unwrap();
        assert_top_level_fields(
            &json,
            &[
                "query",
                "repo_root",
                "repo_rev",
                "latency_ms",
                "coverage",
                "candidates",
                "next_actions",
            ],
        );
    }

    #[test]
    fn map_report_serializes_stable_top_level_fields() {
        let report = MapReport {
            path: "src/search.rs".to_string(),
            role: "source".to_string(),
            index_status: "fresh".to_string(),
            index_path: "C:/repo/.agentgrep/index.json".to_string(),
            repo_rev: Some("abc".to_string()),
            size_bytes: Some(1024),
            modified_unix: Some(1_700_000_000),
            content_hash: Some("deadbeef".to_string()),
            symbols: vec![sample_indexed_symbol("SearchResult", "src/search.rs")],
            outgoing_edges: vec![sample_edge("imports", "src/search.rs", "src/types.rs")],
            incoming_edges: vec![],
            connection_counts: ConnectionCounts {
                outgoing_total: 1,
                incoming_total: 0,
                outgoing_by_type: BTreeMap::from([(String::from("imports"), 1)]),
                incoming_by_type: BTreeMap::new(),
            },
            next_actions: vec!["open src/search.rs".to_string()],
        };

        let json = serde_json::to_value(&report).unwrap();
        assert_top_level_fields(
            &json,
            &[
                "path",
                "role",
                "index_status",
                "index_path",
                "repo_rev",
                "size_bytes",
                "modified_unix",
                "content_hash",
                "symbols",
                "outgoing_edges",
                "incoming_edges",
                "connection_counts",
                "next_actions",
            ],
        );
    }

    #[test]
    fn symbol_report_serializes_stable_top_level_fields() {
        let report = SymbolReport {
            query: "SearchResult".to_string(),
            index_status: "fresh".to_string(),
            match_mode: SymbolMatchMode::Exact,
            matches: vec![SymbolMatch {
                symbol: sample_indexed_symbol("SearchResult", "src/search.rs"),
                file_role: "source".to_string(),
                used_by: vec![sample_reference(
                    "SearchResult",
                    "src/main.rs",
                    "src/search.rs",
                )],
                outgoing_edges: vec![sample_edge("imports", "src/search.rs", "src/types.rs")],
                incoming_edges: vec![],
                next_actions: vec!["open src/search.rs".to_string()],
            }],
            next_actions: vec!["agentgrep map src/search.rs".to_string()],
        };

        let json = serde_json::to_value(&report).unwrap();
        assert_top_level_fields(
            &json,
            &[
                "query",
                "index_status",
                "match_mode",
                "matches",
                "next_actions",
            ],
        );
    }

    #[test]
    fn related_report_serializes_stable_top_level_fields() {
        let report = RelatedReport {
            query: "src/search.rs".to_string(),
            mode: RelatedMode::File,
            index_status: "fresh".to_string(),
            match_mode: None,
            target_file: Some("src/search.rs".to_string()),
            target_role: Some("source".to_string()),
            symbol_matches: vec![],
            related_files: vec![RelatedFile {
                path: "src/types.rs".to_string(),
                role: "source".to_string(),
                score: 1.0,
                confidence: Confidence::High,
                reasons: vec!["imports".to_string()],
            }],
            edges: vec![sample_edge("imports", "src/search.rs", "src/types.rs")],
            symbols: vec![sample_indexed_symbol("SearchResult", "src/search.rs")],
            references: vec![sample_reference(
                "SearchResult",
                "src/main.rs",
                "src/search.rs",
            )],
            next_actions: vec!["agentgrep blast src/search.rs".to_string()],
        };

        let json = serde_json::to_value(&report).unwrap();
        assert_top_level_fields(
            &json,
            &[
                "query",
                "mode",
                "index_status",
                "related_files",
                "edges",
                "symbols",
                "references",
                "next_actions",
            ],
        );
    }

    #[test]
    fn blast_report_serializes_stable_top_level_fields() {
        let report = BlastReport {
            query: "src/search.rs".to_string(),
            mode: BlastMode::File,
            index_status: "fresh".to_string(),
            risk_level: BlastRiskLevel::Medium,
            risk_reasons: vec!["imports are present".to_string()],
            impacted_files: vec![BlastImpactedFile {
                path: "src/types.rs".to_string(),
                role: "source".to_string(),
                score: 1.0,
                confidence: Confidence::High,
                context: BlastImpactContext::Production,
                reasons: vec!["imports".to_string()],
            }],
            affected_symbols: vec![sample_indexed_symbol("SearchResult", "src/search.rs")],
            references: vec![sample_reference(
                "SearchResult",
                "src/main.rs",
                "src/search.rs",
            )],
            suggested_inspection_order: vec!["src/search.rs".to_string()],
            next_actions: vec!["agentgrep related src/search.rs".to_string()],
        };

        let json = serde_json::to_value(&report).unwrap();
        assert_top_level_fields(
            &json,
            &[
                "query",
                "mode",
                "index_status",
                "risk_level",
                "risk_reasons",
                "impacted_files",
                "affected_symbols",
                "references",
                "suggested_inspection_order",
                "next_actions",
            ],
        );
    }
}
