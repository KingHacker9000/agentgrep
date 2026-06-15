use serde::Serialize;

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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
