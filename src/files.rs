use anyhow::Result;

use crate::index::RepoIndex;
use crate::repo::RepoInfo;
use crate::types::{FileMatch, FilesReport};

pub fn build_report(repo: &RepoInfo, pattern: &str, index: &RepoIndex) -> Result<FilesReport> {
    let pat_lower = pattern.to_lowercase();
    // Try glob-style matching first; fall back to plain substring.
    let use_glob = pat_lower.contains('*') || pat_lower.contains('?');

    let glob_matcher = if use_glob {
        let g = globset::Glob::new(&pat_lower)
            .ok()
            .and_then(|g| g.compile_matcher().into());
        g
    } else {
        None
    };

    let mut matches: Vec<FileMatch> = index
        .files
        .iter()
        .filter(|f| {
            let path_lower = f.path.replace('\\', "/").to_lowercase();
            if let Some(ref gm) = glob_matcher {
                gm.is_match(&path_lower)
                    || path_lower
                        .rsplit('/')
                        .next()
                        .map(|name| gm.is_match(name))
                        .unwrap_or(false)
            } else {
                // Substring match against full path or just the basename
                let basename = path_lower.rsplit('/').next().unwrap_or(&path_lower);
                path_lower.contains(&pat_lower) || basename.contains(&pat_lower)
            }
        })
        .map(|f| FileMatch {
            path: f.path.clone(),
            role: f.role.to_string(),
        })
        .collect();

    // Sort: source files first, then by path
    matches.sort_by(|a, b| {
        let a_src = (a.role != "source") as u8;
        let b_src = (b.role != "source") as u8;
        a_src.cmp(&b_src).then_with(|| a.path.cmp(&b.path))
    });

    Ok(FilesReport {
        pattern: pattern.to_string(),
        repo_root: crate::repo::display_path(&repo.root),
        total_indexed: index.files.len(),
        matches,
    })
}

pub fn write_report(report: &FilesReport, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(report)?);
        return Ok(());
    }

    println!(
        "Files matching {:?}  ({}/{} indexed)",
        report.pattern,
        report.matches.len(),
        report.total_indexed
    );
    if report.matches.is_empty() {
        println!("  (no matches)");
        return Ok(());
    }
    for m in &report.matches {
        println!("  [{:8}]  {}", m.role, m.path);
    }
    Ok(())
}
