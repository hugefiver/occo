use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

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

/// Returns the raw `git status --porcelain` output for change detection.
/// Two calls returning different strings means working tree changed.
pub fn get_working_tree_status(repo_path: &Path) -> Result<String> {
    run_git(repo_path, &["status", "--porcelain"])
}

/// Check which paths are ignored by .gitignore rules.
/// Uses `git check-ignore --stdin` for efficient batch checking.
/// Returns the set of ignored paths as normalized forward-slash strings.
pub fn check_ignored(repo_path: &Path, paths: &[PathBuf]) -> Result<HashSet<String>> {
    if paths.is_empty() {
        return Ok(HashSet::new());
    }

    let mut child = Command::new("git")
        .args(["check-ignore", "--no-index", "--stdin"])
        .current_dir(repo_path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("failed to spawn git check-ignore")?;

    {
        let stdin = child
            .stdin
            .as_mut()
            .context("failed to open stdin for git check-ignore")?;
        for path in paths {
            writeln!(stdin, "{}", path.to_string_lossy().replace('\\', "/"))?;
        }
    }

    let output = child
        .wait_with_output()
        .context("git check-ignore failed")?;

    // Exit code: 0 = some paths matched, 1 = no paths matched, >1 = error
    match output.status.code() {
        Some(0) | Some(1) => {}
        _ => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("git check-ignore failed: {}", stderr.trim());
        }
    }

    let stdout =
        String::from_utf8(output.stdout).context("git check-ignore output was not UTF-8")?;
    Ok(stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| l.replace('\\', "/"))
        .collect())
}
