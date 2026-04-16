use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::warn;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilesetState {
    pub head: String,
    pub checkpoint: String,
    pub repo_root: String,
    pub indexed_at: u64,
    pub files_indexed: usize,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct StateFile {
    pub filesets: HashMap<String, FilesetState>,
}

fn state_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("failed to resolve home directory")?;
    Ok(home
        .join(".local")
        .join("share")
        .join("coindex")
        .join("state.json"))
}

impl StateFile {
    pub fn load() -> Self {
        let path = match state_path() {
            Ok(p) => p,
            Err(e) => {
                warn!(error = %e, "cannot determine state path, using empty state");
                return Self::default();
            }
        };

        if !path.exists() {
            return Self::default();
        }

        match std::fs::read_to_string(&path) {
            Ok(raw) => serde_json::from_str(&raw).unwrap_or_else(|e| {
                warn!(path = %path.display(), error = %e, "corrupt state file, resetting");
                Self::default()
            }),
            Err(e) => {
                warn!(path = %path.display(), error = %e, "cannot read state file");
                Self::default()
            }
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = state_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, json)
            .with_context(|| format!("failed to write {}", path.display()))?;
        Ok(())
    }

    pub fn get(&self, fileset_name: &str) -> Option<&FilesetState> {
        self.filesets.get(fileset_name)
    }

    pub fn update(
        &mut self,
        fileset_name: &str,
        head: &str,
        checkpoint: &str,
        repo_root: &str,
        files_indexed: usize,
    ) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.filesets.insert(
            fileset_name.to_string(),
            FilesetState {
                head: head.to_string(),
                checkpoint: checkpoint.to_string(),
                repo_root: repo_root.to_string(),
                indexed_at: now,
                files_indexed,
            },
        );
    }

    pub fn remove(&mut self, fileset_name: &str) {
        self.filesets.remove(fileset_name);
    }
}
