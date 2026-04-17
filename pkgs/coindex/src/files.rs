use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Serialize;

use crate::diff;
use crate::filter::{self, SkipReason};
use crate::output::OutputFormat;

#[derive(Debug, Clone, Serialize)]
pub struct FileEntry {
    pub path: String,
    pub status: FileStatus,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", content = "reason")]
#[serde(rename_all = "snake_case")]
pub enum FileStatus {
    Included,
    Skipped(SkipReason),
}

#[derive(Debug, Clone, Serialize)]
pub struct FilesSummary {
    pub total: usize,
    pub included: usize,
    pub skipped: usize,
    pub files: Vec<FileEntry>,
}

pub fn run_files(
    paths: Vec<PathBuf>,
    no_ignore: bool,
    dirty: bool,
    thorough: bool,
    tree: bool,
    depth: Option<usize>,
    format: OutputFormat,
) -> Result<()> {
    let repo_root = diff::get_repo_root(&paths[0])?;

    let mut all_tracked = diff::get_all_tracked_files(&repo_root)?;

    if dirty {
        let dirty_files = diff::get_dirty_files(&repo_root)?;
        let existing: HashSet<PathBuf> = all_tracked.iter().cloned().collect();
        for f in dirty_files {
            if !existing.contains(&f) {
                all_tracked.push(f);
            }
        }
    }

    let ignored = if no_ignore {
        HashSet::new()
    } else {
        diff::check_ignored(&repo_root, &all_tracked)?
    };

    let git_binaries = if thorough {
        diff::get_git_binary_files(&repo_root)?
    } else {
        HashSet::new()
    };

    let filter_paths: Vec<String> = if paths.len() == 1 && paths[0] == Path::new(".") {
        vec![]
    } else {
        paths
            .iter()
            .map(|p| {
                let abs = if p.is_absolute() {
                    p.clone()
                } else {
                    std::env::current_dir().unwrap_or_default().join(p)
                };
                abs.strip_prefix(&repo_root)
                    .unwrap_or(p.as_path())
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect()
    };

    let mut entries = Vec::new();

    for relative in &all_tracked {
        let normalized = relative.to_string_lossy().replace('\\', "/");

        if !filter_paths.is_empty()
            && !filter_paths
                .iter()
                .any(|fp| normalized.starts_with(fp) || normalized == *fp)
        {
            continue;
        }

        let status = classify_file(&repo_root, relative, &normalized, &ignored, &git_binaries);
        entries.push(FileEntry {
            path: normalized,
            status,
        });
    }

    entries.sort_by(|a, b| a.path.cmp(&b.path));

    let included = entries
        .iter()
        .filter(|e| matches!(e.status, FileStatus::Included))
        .count();
    let summary = FilesSummary {
        total: entries.len(),
        included,
        skipped: entries.len() - included,
        files: entries,
    };

    if tree {
        print_tree(&summary, depth, format);
    } else {
        print_list(&summary, format);
    }

    Ok(())
}

fn classify_file(
    repo_root: &Path,
    relative: &Path,
    normalized: &str,
    ignored: &HashSet<String>,
    git_binaries: &HashSet<String>,
) -> FileStatus {
    if ignored.contains(normalized) {
        return FileStatus::Skipped(SkipReason::Gitignored);
    }

    if git_binaries.contains(normalized) {
        return FileStatus::Skipped(SkipReason::GitBinary);
    }

    let absolute = repo_root.join(relative);
    let metadata = match std::fs::metadata(&absolute) {
        Ok(meta) => meta,
        Err(_) => return FileStatus::Skipped(SkipReason::EmptyFile),
    };

    if !metadata.is_file() {
        return FileStatus::Skipped(SkipReason::EmptyFile);
    }

    if let Some(reason) = filter::classify_path(relative, metadata.len()) {
        return FileStatus::Skipped(reason);
    }

    let bytes = match std::fs::read(&absolute) {
        Ok(content) => content,
        Err(_) => return FileStatus::Skipped(SkipReason::EmptyFile),
    };

    if let Some(reason) = filter::classify_content(&bytes) {
        return FileStatus::Skipped(reason);
    }

    FileStatus::Included
}

fn print_list(summary: &FilesSummary, format: OutputFormat) {
    match format {
        OutputFormat::Json => {
            println!(
                "{}",
                serde_json::to_string_pretty(summary).unwrap_or_default()
            );
        }
        OutputFormat::Markdown => {
            println!(
                "## Files ({} included, {} skipped)\n",
                summary.included, summary.skipped
            );
            println!("| Path | Status |");
            println!("|------|--------|");
            for entry in &summary.files {
                let status_str = match &entry.status {
                    FileStatus::Included => "✓".to_string(),
                    FileStatus::Skipped(r) => format!("✗ {r}"),
                };
                println!("| `{}` | {} |", entry.path, status_str);
            }
        }
        OutputFormat::Plain => {
            for entry in &summary.files {
                match &entry.status {
                    FileStatus::Included => println!("  + {}", entry.path),
                    FileStatus::Skipped(r) => println!("  - {} ({})", entry.path, r),
                }
            }
            println!(
                "\n{} total: {} included, {} skipped",
                summary.total, summary.included, summary.skipped
            );
        }
    }
}

fn print_tree(summary: &FilesSummary, max_depth: Option<usize>, format: OutputFormat) {
    if format == OutputFormat::Json {
        println!(
            "{}",
            serde_json::to_string_pretty(summary).unwrap_or_default()
        );
        return;
    }

    let tree = build_tree(&summary.files);

    let is_md = format == OutputFormat::Markdown;
    if is_md {
        println!(
            "## Files ({} included, {} skipped)\n",
            summary.included, summary.skipped
        );
        println!("```");
    }

    render_tree_node(&tree, "", true, 0, max_depth);

    if is_md {
        println!("```");
    } else {
        println!(
            "\n{} total: {} included, {} skipped",
            summary.total, summary.included, summary.skipped
        );
    }
}

#[derive(Debug, Default)]
struct TreeNode {
    children_dirs: BTreeMap<String, TreeNode>,
    files: Vec<(String, FileStatus)>,
}

impl TreeNode {
    fn count(&self) -> (usize, usize) {
        let mut inc = 0usize;
        let mut skip = 0usize;
        for (_, s) in &self.files {
            match s {
                FileStatus::Included => inc += 1,
                FileStatus::Skipped(_) => skip += 1,
            }
        }
        for (_, child) in &self.children_dirs {
            let (ci, cs) = child.count();
            inc += ci;
            skip += cs;
        }
        (inc, skip)
    }
}

fn build_tree(files: &[FileEntry]) -> TreeNode {
    let mut root = TreeNode::default();
    for entry in files {
        let parts: Vec<&str> = entry.path.split('/').collect();
        let mut node = &mut root;
        for (i, part) in parts.iter().enumerate() {
            if i == parts.len() - 1 {
                node.files.push((part.to_string(), entry.status.clone()));
            } else {
                node = node.children_dirs.entry(part.to_string()).or_default();
            }
        }
    }
    root
}

fn render_tree_node(
    node: &TreeNode,
    prefix: &str,
    is_root: bool,
    current_depth: usize,
    max_depth: Option<usize>,
) {
    let total_children = node.children_dirs.len() + node.files.len();
    let mut idx = 0;

    let dirs: Vec<_> = node.children_dirs.iter().collect();
    for (name, child) in &dirs {
        idx += 1;
        let is_last = idx == total_children;
        let connector = if is_root {
            ""
        } else if is_last {
            "└── "
        } else {
            "├── "
        };
        let child_prefix = if is_root {
            String::new()
        } else if is_last {
            format!("{prefix}    ")
        } else {
            format!("{prefix}│   ")
        };

        if let Some(max) = max_depth {
            if current_depth >= max {
                let (inc, skip) = child.count();
                println!("{prefix}{connector}{name}/ ({inc} included, {skip} skipped)");
                continue;
            }
        }

        println!("{prefix}{connector}{name}/");
        render_tree_node(child, &child_prefix, false, current_depth + 1, max_depth);
    }

    for (name, status) in &node.files {
        idx += 1;
        let is_last = idx == total_children;
        let connector = if is_root {
            ""
        } else if is_last {
            "└── "
        } else {
            "├── "
        };
        let label = match status {
            FileStatus::Included => format!("{name}"),
            FileStatus::Skipped(r) => format!("{name} ({r})"),
        };
        println!("{prefix}{connector}{label}");
    }
}
