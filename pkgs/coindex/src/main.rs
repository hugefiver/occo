mod auth;
mod client;
mod diff;
mod filter;
mod types;
mod wasm;

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Result, anyhow};
use base64::Engine as _;
use clap::{Parser, Subcommand};
use sha1::{Sha1, Digest};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use crate::auth::{auth_status, get_token};
use crate::client::IngestClient;
use crate::types::{
    CreateIngestRequest, DeleteFilesetRequest, FinalizeRequest, SearchRequest, UploadDocumentRequest,
};
use crate::wasm::BlackbirdWasm;

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

struct FileData {
    relative_path: String,
    content: Vec<u8>,
    doc_sha: [u8; 20],
}

fn collect_file_data(repo_root: &Path, files: Vec<PathBuf>) -> Vec<FileData> {
    let mut result = Vec::new();

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

        let rel_path = relative_to_api_path(&relative);
        let doc_sha = BlackbirdWasm::get_doc_sha(&rel_path, &bytes);

        result.push(FileData {
            relative_path: rel_path,
            content: bytes,
            doc_sha,
        });
    }

    result
}

async fn run_index(path: PathBuf, checkpoint: Option<String>) -> Result<()> {
    let b64 = base64::engine::general_purpose::STANDARD;
    let started = Instant::now();
    let token = get_token().await?;
    let client = IngestClient::new(token)?;

    let repo_root = diff::get_repo_root(&path)?;
    let fileset_name = repo_root
        .file_name()
        .and_then(|name| name.to_str())
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow!("failed to derive fileset name from {}", repo_root.display()))?;
    let git_head = diff::get_current_head(&repo_root)?;

    let candidates = match checkpoint.as_deref() {
        Some(previous) => {
            let deleted = diff::get_deleted_files(&repo_root, previous, &git_head)?;
            if !deleted.is_empty() {
                info!(deleted = deleted.len(), "detected deleted files");
            }
            diff::get_changed_files(&repo_root, previous, &git_head)?
        }
        None => diff::get_all_tracked_files(&repo_root)?,
    };

    // Collect file data with doc_sha (pure Rust)
    let file_data = collect_file_data(&repo_root, candidates);
    let files_indexed = file_data.len();
    info!(files = files_indexed, "collected files for indexing");

    if file_data.is_empty() {
        println!("no files to index");
        return Ok(());
    }

    // Compute doc_shas array
    let doc_shas: Vec<[u8; 20]> = file_data.iter().map(|f| f.doc_sha).collect();

    // Checkpoint = SHA-1 of all docShas concatenated (matches VS Code client)
    let mut checkpoint_hasher = Sha1::new();
    for sha in &doc_shas {
        checkpoint_hasher.update(sha);
    }
    let new_checkpoint = b64.encode(checkpoint_hasher.finalize());

    // Initialize WASM runtime for geo_filter + coded_symbols
    info!("initializing blackbird WASM runtime");
    let mut bb = BlackbirdWasm::new()?;

    for (i, sha) in doc_shas.iter().enumerate() {
        tracing::debug!(idx = i, sha = %hex::encode(sha), "doc_sha");
    }

    // Compute geo_filter
    let geo_filter_bytes = bb.compute_geo_filter(&doc_shas)?;
    let geo_filter = b64.encode(&geo_filter_bytes);
    tracing::debug!(hex = %hex::encode(&geo_filter_bytes), "geo_filter raw");
    info!(bytes = geo_filter_bytes.len(), "computed geo_filter");

    // Compute initial coded_symbols (range [0, 1))
    let initial_symbols = bb.create_coded_symbols(&doc_shas, 0, 1)?;
    let coded_symbols: Vec<String> = initial_symbols.iter().map(|s| b64.encode(s)).collect();
    for (i, s) in initial_symbols.iter().enumerate() {
        tracing::debug!(idx = i, hex = %hex::encode(s), "coded_symbol");
    }
    info!(count = coded_symbols.len(), "computed initial coded_symbols");

    // Create ingest
    let ingest = client
        .create_ingest(CreateIngestRequest {
            fileset_name: fileset_name.clone(),
            new_checkpoint: new_checkpoint.clone(),
            geo_filter,
            coded_symbols,
        })
        .await?;

    if ingest.ingest_id.is_empty()
        && ingest.coded_symbol_range.start == 0
        && ingest.coded_symbol_range.end == 0
    {
        println!("already indexed: {fileset_name} at {new_checkpoint}");
        return Ok(());
    }

    let ingest_id = ingest.ingest_id;
    info!(ingest_id = %ingest_id, "ingest created");

    // Upload remaining coded_symbols in batches
    let mut next_range = Some(ingest.coded_symbol_range);
    while let Some(range) = next_range.take() {
        if range.start >= range.end {
            break;
        }
        let symbols = bb.create_coded_symbols(&doc_shas, range.start as u32, range.end as u32)?;
        let encoded: Vec<String> = symbols.iter().map(|s| b64.encode(s)).collect();
        info!(start = range.start, end = range.end, count = encoded.len(), "uploading coded_symbols");

        let resp = client
            .upload_symbols(crate::types::UploadSymbolsRequest {
                ingest_id: ingest_id.clone(),
                coded_symbols: encoded,
                coded_symbol_range: range,
            })
            .await?;

        next_range = resp.next_coded_symbol_range;
    }

    // Build doc_id → FileData lookup (base64-encoded doc_sha)
    let doc_map: std::collections::HashMap<String, &FileData> = file_data
        .iter()
        .map(|f| (b64.encode(f.doc_sha), f))
        .collect();

    // Fetch batches and upload documents
    let mut page_token = String::new();
    let mut uploaded = 0usize;
    loop {
        let batch = client
            .get_batch(crate::types::BatchRequest {
                ingest_id: ingest_id.clone(),
                page_token: page_token.clone(),
            })
            .await?;

        let mut uploads = Vec::new();
        for doc_id in &batch.doc_ids {
            if let Some(fd) = doc_map.get(doc_id) {
                uploads.push(UploadDocumentRequest {
                    ingest_id: ingest_id.clone(),
                    content: b64.encode(&fd.content),
                    file_path: fd.relative_path.clone(),
                    doc_id: doc_id.clone(),
                });
            } else {
                // Deletion marker: empty content + empty path
                uploads.push(UploadDocumentRequest {
                    ingest_id: ingest_id.clone(),
                    content: String::new(),
                    file_path: String::new(),
                    doc_id: doc_id.clone(),
                });
            }
        }

        uploaded += uploads.len();
        client.upload_documents_concurrent(uploads).await?;

        match batch.next_page_token {
            Some(token) if !token.is_empty() => page_token = token,
            _ => break,
        }
    }
    info!(uploaded, "documents uploaded");

    // Finalize
    client
        .finalize(FinalizeRequest {
            ingest_id: ingest_id,
        })
        .await?;

    println!(
        "indexed {} files ({} uploaded) in {:?} (fileset: {}, checkpoint: {})",
        files_indexed,
        uploaded,
        started.elapsed(),
        fileset_name,
        &new_checkpoint[..14],
    );

    Ok(())
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
        let lang = result
            .location
            .language
            .as_ref()
            .map(|l| l.name.as_str())
            .unwrap_or("unknown");
        println!(
            "{}. {} [{}] ({:.4})",
            index + 1,
            result.location.path,
            lang,
            result.distance,
        );
        if let Some(ref lr) = result.chunk.line_range {
            println!("   lines: {}-{}", lr.start, lr.end);
        }

        let text = result.chunk.text.replace('\n', " ");
        let snippet = if text.len() > 200 {
            format!("{}...", &text[..200])
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
