use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use tracing::{error, info, warn};

use crate::auth::get_token;
use crate::diff;
use crate::run_index_core;
use crate::state;

pub async fn run_daemon(
    path: PathBuf,
    interval_secs: u64,
    no_ignore: bool,
    interactive: bool,
) -> Result<()> {
    let repo_root = diff::get_repo_root(&path)?;
    let fileset_name = repo_root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_string();
    info!(root = %repo_root.display(), interval = interval_secs, "daemon started");

    let saved = state::StateFile::load();
    let mut last_head: Option<String> = saved.get(&fileset_name).map(|s| s.head.clone());
    if let Some(head) = &last_head {
        info!(head = %head, "resumed from saved state");
    }
    let mut last_tree_status: Option<String> = None;

    loop {
        let current_head = match diff::get_current_head(&repo_root) {
            Ok(h) => h,
            Err(e) => {
                warn!(error = %e, "failed to read HEAD, retrying next cycle");
                tokio::time::sleep(Duration::from_secs(interval_secs)).await;
                continue;
            }
        };

        let current_tree = match diff::get_working_tree_status(&repo_root) {
            Ok(s) => s,
            Err(e) => {
                warn!(error = %e, "failed to read working tree status");
                String::new()
            }
        };

        let head_changed = last_head.as_ref() != Some(&current_head);
        let tree_changed = last_tree_status.as_ref() != Some(&current_tree);

        if !head_changed && !tree_changed {
            tokio::time::sleep(Duration::from_secs(interval_secs)).await;
            continue;
        }

        // Tree changes (uncommitted edits, new files) require a full index because
        // incremental mode only diffs between commits and would miss working-tree state.
        // Only use incremental when HEAD changed without tree changes.
        let since = if head_changed && !tree_changed {
            last_head.clone()
        } else {
            None
        };

        info!(
            from = since.as_deref().unwrap_or("(full)"),
            to = %current_head,
            head_changed,
            tree_changed,
            "change detected, indexing"
        );

        let token = match get_token(interactive).await {
            Ok(t) => t,
            Err(e) => {
                error!(error = %e, "failed to get token");
                tokio::time::sleep(Duration::from_secs(interval_secs)).await;
                continue;
            }
        };

        match run_index_core(token, path.clone(), since, no_ignore).await {
            Ok(result) => {
                last_head = Some(result.head);
                last_tree_status = Some(current_tree);
                info!(
                    files = result.files_indexed,
                    uploaded = result.files_uploaded,
                    ?result.elapsed,
                    "index cycle complete"
                );
            }
            Err(e) => {
                error!(error = %e, "index cycle failed");
            }
        }

        tokio::time::sleep(Duration::from_secs(interval_secs)).await;
    }
}
