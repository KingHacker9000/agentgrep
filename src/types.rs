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
}

#[derive(Debug, Clone, Serialize)]
pub struct FileCandidate {
    pub path: String,
    pub kind: String,
    pub role: String,
    pub score: f64,
    pub confidence: Confidence,
    pub line_ranges: Vec<LineRange>,
    pub snippets: Vec<Snippet>,
    pub evidence: Vec<Evidence>,
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
        }
    }

    pub fn finalize(mut self, displayed_candidate_count: usize, candidate_limit: usize) -> Self {
        self.displayed_candidate_count = displayed_candidate_count;
        self.candidate_limit = candidate_limit;
        self.limited = displayed_candidate_count < self.raw_candidate_file_count;
        self
    }
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum SymbolKind {
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Visibility {
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexedSymbol {
    pub name: String,
    pub kind: SymbolKind,
    pub file_path: String,
    pub line_number: usize,
    pub visibility: Visibility,
    pub signature: Option<String>,
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
    }
}
