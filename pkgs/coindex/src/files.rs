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

#[allow(clippy::too_many_arguments)]
pub fn run_files(
    paths: Vec<PathBuf>,
    no_ignore: bool,
    dirty: bool,
    thorough: bool,
    tree: bool,
    depth: Option<usize>,
    include_ignored: bool,
    compact: bool,
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

        if ignored.contains(&normalized)
            && !include_ignored {
                continue;
            }

        let status = classify_file(&repo_root, relative, &normalized, &ignored, &git_binaries);

        if !include_ignored
            && matches!(status, FileStatus::Skipped(SkipReason::SkipDir)) {
                continue;
            }

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
        print_tree(&summary, depth, compact, format);
    } else {
        print_list(&summary, compact, format);
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

fn collect_display_entries(node: &TreeNode, prefix: &str, compact: bool) -> Vec<DisplayEntry> {
    let mut result = Vec::new();
    for (name, child) in &node.children_dirs {
        let child_path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        if compact && child.all_skipped() {
            result.push(DisplayEntry::CollapsedDir {
                path: child_path,
                count: child.total_files(),
            });
        } else {
            result.extend(collect_display_entries(child, &child_path, compact));
        }
    }
    for (name, status) in &node.files {
        let file_path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        result.push(DisplayEntry::File {
            path: file_path,
            status: status.clone(),
        });
    }
    result
}

fn print_list(summary: &FilesSummary, compact: bool, format: OutputFormat) {
    match format {
        OutputFormat::Json => {
            println!(
                "{}",
                serde_json::to_string_pretty(summary).unwrap_or_default()
            );
        }
        OutputFormat::Markdown => {
            let tree = build_tree(&summary.files);
            let display = collect_display_entries(&tree, "", compact);
            println!(
                "## Files ({} included, {} skipped)\n",
                summary.included, summary.skipped
            );
            println!("| Path | Status |");
            println!("|------|--------|");
            for entry in &display {
                match entry {
                    DisplayEntry::File { path, status } => {
                        let status_str = match status {
                            FileStatus::Included => "✓".to_string(),
                            FileStatus::Skipped(r) => format!("✗ {r}"),
                        };
                        println!("| `{path}` | {status_str} |");
                    }
                    DisplayEntry::CollapsedDir { path, count } => {
                        println!("| `{path}/` | ✗ {count} files skipped |");
                    }
                }
            }
        }
        OutputFormat::Plain => {
            let tree = build_tree(&summary.files);
            let display = collect_display_entries(&tree, "", compact);
            for entry in &display {
                match entry {
                    DisplayEntry::File { path, status } => match status {
                        FileStatus::Included => println!("  + {path}"),
                        FileStatus::Skipped(r) => println!("  - {path} ({r})"),
                    },
                    DisplayEntry::CollapsedDir { path, count } => {
                        println!("  - {path}/ ({count} files skipped)");
                    }
                }
            }
            println!(
                "\n{} total: {} included, {} skipped",
                summary.total, summary.included, summary.skipped
            );
        }
    }
}

fn print_tree(
    summary: &FilesSummary,
    max_depth: Option<usize>,
    compact: bool,
    format: OutputFormat,
) {
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

    render_tree_node(&tree, "", true, 0, max_depth, compact);

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

enum DisplayEntry {
    File { path: String, status: FileStatus },
    CollapsedDir { path: String, count: usize },
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
        for child in self.children_dirs.values() {
            let (ci, cs) = child.count();
            inc += ci;
            skip += cs;
        }
        (inc, skip)
    }

    fn all_skipped(&self) -> bool {
        (!self.files.is_empty() || !self.children_dirs.is_empty())
            && self
                .files
                .iter()
                .all(|(_, s)| matches!(s, FileStatus::Skipped(_)))
            && self.children_dirs.values().all(|c| c.all_skipped())
    }

    fn total_files(&self) -> usize {
        self.files.len()
            + self
                .children_dirs
                .values()
                .map(|c| c.total_files())
                .sum::<usize>()
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
    compact: bool,
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

        if compact && child.all_skipped() {
            let total = child.total_files();
            println!("{prefix}{connector}{name}/ ({total} files skipped)");
            continue;
        }

        if let Some(max) = max_depth
            && current_depth >= max {
                let (inc, skip) = child.count();
                println!("{prefix}{connector}{name}/ ({inc} included, {skip} skipped)");
                continue;
            }

        println!("{prefix}{connector}{name}/");
        render_tree_node(
            child,
            &child_prefix,
            false,
            current_depth + 1,
            max_depth,
            compact,
        );
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
            FileStatus::Included => name.to_string(),
            FileStatus::Skipped(r) => format!("{name} ({r})"),
        };
        println!("{prefix}{connector}{label}");
    }
}
