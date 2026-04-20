use std::path::Path;
#[cfg(feature = "vscode-local")]
use std::path::PathBuf;

#[cfg(feature = "vscode-local")]
use anyhow::Context;
use anyhow::{bail, Result};

use crate::types::SearchResult;
#[cfg(feature = "vscode-local")]
use crate::types::{Chunk, ChunkRange, Location};

#[cfg(feature = "vscode-local")]
fn vscode_workspace_storage_root() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("Code").join("User").join("workspaceStorage"))
}

#[cfg(feature = "vscode-local")]
fn find_workspace_db(storage_root: &Path, repo_root: &Path) -> Result<PathBuf> {
    let repo_str = repo_root.to_string_lossy();

    for entry in std::fs::read_dir(storage_root).context("failed to read workspace storage dir")? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }

        let ws_json = entry.path().join("workspace.json");
        if !ws_json.exists() {
            continue;
        }

        let content = std::fs::read_to_string(&ws_json)?;
        let json: serde_json::Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(_) => continue,
        };

        if let Some(folder) = json.get("folder").and_then(|v| v.as_str()) {
            let folder_path = folder.strip_prefix("file:///").unwrap_or(folder);

            if folder_path.contains(&*repo_str) || repo_str.contains(folder_path) {
                let db = entry
                    .path()
                    .join("GitHub.copilot-chat")
                    .join("workspace-chunks.db");
                if db.exists() {
                    return Ok(db);
                }
            }
        }
    }

    bail!(
        "no VS Code workspace-chunks.db found for {}",
        repo_root.display()
    )
}

#[cfg(feature = "vscode-local")]
pub fn search_vscode_local(repo_root: &Path, query: &str, limit: u32) -> Result<Vec<SearchResult>> {
    use rusqlite::Connection;

    let storage_root = vscode_workspace_storage_root()
        .context("could not determine VS Code workspace storage path")?;

    if !storage_root.exists() {
        bail!(
            "VS Code workspace storage not found at {}",
            storage_root.display()
        );
    }

    let db_path = find_workspace_db(&storage_root, repo_root)?;

    let conn = Connection::open_with_flags(&db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("failed to open {}", db_path.display()))?;

    // The schema may vary across VS Code versions. Try the known column layout
    // from workspaceChunkAndEmbeddingCache.ts:
    //   Files(id, uri, contentVersionId)
    //   FileChunks(id, fileId, chunkHash, chunkText, startLine, endLine, embedding)
    let mut stmt = conn
        .prepare(
            "SELECT fc.chunkText, fc.chunkHash, f.uri, fc.startLine, fc.endLine \
             FROM FileChunks fc \
             JOIN Files f ON fc.fileId = f.id \
             WHERE fc.chunkText LIKE ?1 \
             LIMIT ?2",
        )
        .context("failed to prepare search query — schema may have changed")?;

    let pattern = format!("%{query}%");
    let rows = stmt
        .query_map(rusqlite::params![pattern, limit], |row| {
            let uri: String = row.get(2)?;
            let path = uri.strip_prefix("file:///").unwrap_or(&uri).to_string();

            Ok(SearchResult {
                location: Location {
                    fileset: None,
                    checkpoint: None,
                    doc_id: None,
                    path,
                    language: None,
                    commit_sha: None,
                    repo: None,
                },
                distance: 0.0,
                text: None,
                chunk: Chunk {
                    hash: row.get::<_, String>(1)?,
                    text: row.get::<_, String>(0)?,
                    line_range: Some(ChunkRange {
                        start: row.get(3)?,
                        end: row.get(4)?,
                    }),
                    range: None,
                },
            })
        })
        .context("search query failed")?;

    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }
    Ok(results)
}

#[cfg(not(feature = "vscode-local"))]
pub fn search_vscode_local(
    _repo_root: &Path,
    _query: &str,
    _limit: u32,
) -> Result<Vec<SearchResult>> {
    bail!(
        "VS Code local index support requires the 'vscode-local' feature. \
         Rebuild with: cargo build --features vscode-local"
    )
}
