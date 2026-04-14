use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

pub fn get_current_head(repo_path: &Path) -> Result<String> {
    let output = run_git(repo_path, &["rev-parse", "HEAD"])?;
    Ok(output.trim().to_string())
}

pub fn get_changed_files(
    repo_path: &Path,
    from_commit: &str,
    to_commit: &str,
) -> Result<Vec<PathBuf>> {
    let range = format!("{from_commit}..{to_commit}");
    let output = run_git(
        repo_path,
        &["diff", "--name-only", "--diff-filter=ACMR", &range],
    )?;
    Ok(lines_to_paths(&output))
}

pub fn get_deleted_files(
    repo_path: &Path,
    from_commit: &str,
    to_commit: &str,
) -> Result<Vec<PathBuf>> {
    let range = format!("{from_commit}..{to_commit}");
    let output = run_git(
        repo_path,
        &["diff", "--name-only", "--diff-filter=D", &range],
    )?;
    Ok(lines_to_paths(&output))
}

pub fn get_all_tracked_files(repo_path: &Path) -> Result<Vec<PathBuf>> {
    let output = run_git(repo_path, &["ls-files"])?;
    Ok(lines_to_paths(&output))
}

pub fn get_repo_root(path: &Path) -> Result<PathBuf> {
    let output = run_git(path, &["rev-parse", "--show-toplevel"])?;
    Ok(PathBuf::from(output.trim()))
}

fn run_git(repo_path: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo_path)
        .output()
        .with_context(|| format!("failed to run git {:?}", args))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        bail!("git {:?} failed: {}", args, stderr);
    }

    String::from_utf8(output.stdout).context("git output was not valid UTF-8")
}

fn lines_to_paths(output: &str) -> Vec<PathBuf> {
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .collect()
}
