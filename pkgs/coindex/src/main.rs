mod auth;
mod client;
mod diff;
mod filter;
mod types;

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Result, anyhow};
use base64::Engine as _;
use clap::{Parser, Subcommand};
use sha2::{Digest, Sha256};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use crate::auth::{auth_status, get_token};
use crate::client::IngestClient;
use crate::types::{
    CreateIngestRequest, DeleteFilesetRequest, FinalizeRequest, SearchRequest, UploadDocumentRequest,
};

#[derive(Parser)]
#[command(name = "coindex", about = "GitHub Copilot External Ingest CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    #[command(about = "Build or update the semantic index for a repository")]
    Index {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long)]
        checkpoint: Option<String>,
    },
    #[command(about = "Search the semantic index")]
    Search {
        query: String,
        #[arg(long)]
        fileset: String,
        #[arg(long, default_value = "10")]
        limit: u32,
    },
    #[command(about = "List indexed filesets and their status")]
    Status,
    #[command(about = "Delete a fileset")]
    Delete {
        fileset: String,
    },
    #[command(about = "Show authentication status")]
    Auth,
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing()?;
    let cli = Cli::parse();

    match cli.command {
        Commands::Index { path, checkpoint } => run_index(path, checkpoint).await,
        Commands::Search {
            query,
            fileset,
            limit,
        } => run_search(query, fileset, limit).await,
        Commands::Status => run_status().await,
        Commands::Delete { fileset } => run_delete(fileset).await,
        Commands::Auth => auth_status().await,
    }
}

fn init_tracing() -> Result<()> {
    let filter = match EnvFilter::try_from_default_env() {
        Ok(value) => value,
        Err(_) => EnvFilter::new("info"),
    };

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .try_init()
        .map_err(|error| anyhow!("failed to initialize tracing: {error}"))?;
    Ok(())
}

async fn run_index(path: PathBuf, checkpoint: Option<String>) -> Result<()> {
    let started = Instant::now();
    let token = get_token().await?;
    let client = IngestClient::new(token)?;

    let repo_root = diff::get_repo_root(&path)?;
    let fileset_name = repo_root
        .file_name()
        .and_then(|name| name.to_str())
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow!("failed to derive fileset name from {}", repo_root.display()))?;
    let new_checkpoint = diff::get_current_head(&repo_root)?;

    let candidates = match checkpoint.as_deref() {
        Some(previous) => {
            let deleted = diff::get_deleted_files(&repo_root, previous, &new_checkpoint)?;
            if !deleted.is_empty() {
                info!(deleted = deleted.len(), "detected deleted files");
            }
            diff::get_changed_files(&repo_root, previous, &new_checkpoint)?
        }
        None => diff::get_all_tracked_files(&repo_root)?,
    };

    let mut uploads = build_upload_requests(&repo_root, candidates)?;
    let files_indexed = uploads.len();

    let ingest = client
        .create_ingest(CreateIngestRequest {
            fileset_name: fileset_name.clone(),
            new_checkpoint: new_checkpoint.clone(),
            geo_filter: "global".to_string(),
            coded_symbols: Vec::new(),
        })
        .await?;

    if ingest.ingest_id.is_empty()
        && ingest.coded_symbol_range.start == 0
        && ingest.coded_symbol_range.end == 0
    {
        println!("already indexed: {fileset_name} at {new_checkpoint}");
        return Ok(());
    }

    for request in &mut uploads {
        request.ingest_id = ingest.ingest_id.clone();
    }

    client.upload_documents_concurrent(uploads).await?;
    client
        .finalize(FinalizeRequest {
            ingest_id: ingest.ingest_id,
        })
        .await?;

    println!(
        "indexed {} files in {:?} (fileset: {}, checkpoint: {})",
        files_indexed,
        started.elapsed(),
        fileset_name,
        new_checkpoint
    );

    Ok(())
}

fn build_upload_requests(repo_root: &Path, files: Vec<PathBuf>) -> Result<Vec<UploadDocumentRequest>> {
    let mut uploads = Vec::new();

    for relative in files {
        let absolute = repo_root.join(&relative);

        let metadata = match std::fs::metadata(&absolute) {
            Ok(meta) => meta,
            Err(error) => {
                warn!(path = %absolute.display(), %error, "skipping file: metadata read failed");
                continue;
            }
        };

        if !metadata.is_file() {
            continue;
        }

        if !filter::can_ingest(&relative, metadata.len()) {
            continue;
        }

        let bytes = match std::fs::read(&absolute) {
            Ok(content) => content,
            Err(error) => {
                warn!(path = %absolute.display(), %error, "skipping file: content read failed");
                continue;
            }
        };

        if !filter::can_ingest_content(&bytes) {
            continue;
        }

        let file_path = relative_to_api_path(&relative);
        let mut hasher = Sha256::new();
        hasher.update(file_path.as_bytes());
        let doc_id = hex::encode(hasher.finalize());

        uploads.push(UploadDocumentRequest {
            ingest_id: String::new(),
            content: base64::engine::general_purpose::STANDARD.encode(bytes),
            file_path,
            doc_id,
        });
    }

    Ok(uploads)
}

fn relative_to_api_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

async fn run_search(query: String, fileset: String, limit: u32) -> Result<()> {
    let token = get_token().await?;
    let client = IngestClient::new(token)?;

    let response = client
        .search(SearchRequest {
            prompt: query,
            scoping_query: format!("fileset:{fileset}"),
            embedding_model: "metis-1024-I16-Binary".to_string(),
            limit,
        })
        .await?;

    if response.results.is_empty() {
        println!("no results");
        return Ok(());
    }

    for (index, result) in response.results.iter().enumerate() {
        println!("{}. {} ({:.6})", index + 1, result.location.path, result.distance);
        if let Some((start, end)) = result.chunk.line_range {
            println!("   lines: {start}-{end}");
        }

        let text = result.text.replace('\n', " ");
        let snippet = if text.len() > 180 {
            format!("{}...", &text[..180])
        } else {
            text
        };
        println!("   {snippet}");
    }

    Ok(())
}

async fn run_status() -> Result<()> {
    let token = get_token().await?;
    let client = IngestClient::new(token)?;
    let response = client.list_filesets().await?;

    if response.filesets.is_empty() {
        println!("no filesets");
        return Ok(());
    }

    println!("{:<36} {:<14} {:<12}", "name", "checkpoint", "status");
    for fileset in response.filesets {
        let checkpoint = if fileset.checkpoint.len() > 14 {
            fileset.checkpoint.chars().take(14).collect::<String>()
        } else {
            fileset.checkpoint
        };
        println!("{:<36} {:<14} {:<12}", fileset.name, checkpoint, fileset.status);
    }
    println!("max_filesets: {}", response.max_filesets);

    Ok(())
}

async fn run_delete(fileset: String) -> Result<()> {
    let token = get_token().await?;
    let client = IngestClient::new(token)?;
    client
        .delete_fileset(DeleteFilesetRequest {
            fileset_name: fileset.clone(),
        })
        .await?;
    println!("deleted fileset: {fileset}");
    Ok(())
}
