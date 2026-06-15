use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct RepoInfo {
    pub root: PathBuf,
    pub rev: Option<String>,
    pub git_dir: Option<PathBuf>,
}

pub fn display_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

pub fn discover() -> Result<RepoInfo> {
    let cwd = std::env::current_dir().context("failed to read current directory")?;
    let git_root = run_git(&cwd, &["rev-parse", "--show-toplevel"])?;

    let root = git_root.map(PathBuf::from).unwrap_or(cwd.clone());
    let rev = run_git(&root, &["rev-parse", "--short", "HEAD"])?;
    let git_dir = run_git(&root, &["rev-parse", "--git-dir"])?;
    let git_dir = git_dir.map(|value| resolve_git_path(&root, &value));

    Ok(RepoInfo { root, rev, git_dir })
}

fn run_git(cwd: &Path, args: &[&str]) -> Result<Option<String>> {
    let output = match Command::new("git").args(args).current_dir(cwd).output() {
        Ok(output) => output,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(err) => return Err(err).context("failed to run git"),
    };

    if !output.status.success() {
        return Ok(None);
    }

    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if value.is_empty() {
        Ok(None)
    } else {
        Ok(Some(value))
    }
}

fn resolve_git_path(root: &Path, value: &str) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        root.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_relative_git_dir_under_repo_root() {
        let root = PathBuf::from("C:/repo/subdir");
        let resolved = resolve_git_path(&root, ".git");
        assert_eq!(resolved, PathBuf::from("C:/repo/subdir/.git"));
    }
}
