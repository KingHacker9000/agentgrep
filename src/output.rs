use anyhow::Result;

use crate::types::FindReport;

pub fn write_find_report(report: &FindReport, json: bool) -> Result<()> {
    if json {
        let rendered = serde_json::to_string_pretty(report)?;
        println!("{rendered}");
        return Ok(());
    }

    if report.candidates.is_empty() {
        println!("No matches found.");
        return Ok(());
    }

    println!("Top candidates:");
    for (index, candidate) in report.candidates.iter().enumerate() {
        println!(
            "{}. {}    role {}    score {:.2}    confidence {}",
            index + 1,
            candidate.path,
            candidate.role,
            candidate.score,
            candidate.confidence
        );

        println!("   Lines: {}", format_line_ranges(&candidate.line_ranges));
        if !candidate.snippets.is_empty() {
            println!("   Snippets:");
            for snippet in candidate.snippets.iter().take(3) {
                println!("   - {}: {}", snippet.line_number, snippet.text);
            }
        }
        println!("   Why: {}", explain(candidate));
        println!();
    }

    if !report.next_actions.is_empty() {
        println!("Next:");
        for action in &report.next_actions {
            println!("- {action}");
        }
        println!();
    }

    println!("Search coverage:");
    println!("- raw rg matches: {}", report.coverage.raw_rg_match_count);
    println!(
        "- raw candidate files: {}",
        report.coverage.raw_candidate_file_count
    );
    println!(
        "- displayed candidates: {}",
        report.coverage.displayed_candidate_count
    );
    println!(
        "- limited: {}",
        if report.coverage.limited { "yes" } else { "no" }
    );
    println!(
        "- match limit per file: {}",
        report.coverage.match_limit_per_file
    );
    println!("- candidate limit: {}", report.coverage.candidate_limit);
    println!(
        "- index used: {}",
        if report.coverage.index_used {
            "true"
        } else {
            "false"
        }
    );
    println!("- index status: {}", report.coverage.index_status);

    Ok(())
}

fn explain(candidate: &crate::types::FileCandidate) -> String {
    let mut parts = candidate
        .evidence
        .iter()
        .take(4)
        .map(|item| item.detail.as_str())
        .collect::<Vec<_>>();

    if candidate.evidence.len() > 4 {
        parts.push("...");
    }

    parts.join("; ")
}

fn format_line_ranges(ranges: &[crate::types::LineRange]) -> String {
    let mut parts = ranges
        .iter()
        .take(5)
        .map(|range| {
            if range.start == range.end {
                range.start.to_string()
            } else {
                format!("{}-{}", range.start, range.end)
            }
        })
        .collect::<Vec<_>>();

    if ranges.len() > 5 {
        parts.push("...".to_string());
    }

    parts.join(", ")
}
