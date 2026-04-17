mod auth;
mod client;
mod daemon;
mod diff;
mod files;
mod filter;
mod output;
mod state;
mod types;
mod wasm;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Result, anyhow};
use base64::Engine as _;
use clap::{Parser, Subcommand};
use sha1::{Digest, Sha1};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use crate::auth::{get_token, run_auth};
use crate::client::IngestClient;
use crate::output::OutputFormat;
use crate::types::{
    CreateIngestRequest, DeleteFilesetRequest, FinalizeRequest, SearchRequest,
    UploadDocumentRequest,
};
use crate::wasm::BlackbirdWasm;

#[derive(Parser)]
#[command(name = "coindex", about = "GitHub Copilot External Ingest CLI")]
struct Cli {
    /// Output as JSON (suppresses log output)
    #[arg(long, global = true)]
    json: bool,
    /// Output as Markdown (suppresses log output)
    #[arg(long, alias = "markdown", global = true)]
    md: bool,
    #[command(subcommand)]
    command: Commands,
}

impl Cli {
    fn output_format(&self) -> OutputFormat {
        if self.json {
            OutputFormat::Json
        } else if self.md {
            OutputFormat::Markdown
        } else {
            OutputFormat::Plain
        }
    }
}

#[derive(Subcommand)]
enum Commands {
    #[command(about = "Build or update the semantic index for a repository")]
    Index {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Git commit ref for incremental indexing (use `head` from previous run)
        #[arg(long, alias = "checkpoint")]
        since: Option<String>,
        #[arg(long, help = "Index files even if they match .gitignore rules")]
        no_ignore: bool,
        #[arg(
            long,
            help = "Include uncommitted working tree changes and untracked files"
        )]
        dirty: bool,
        #[arg(
            long,
            help = "Use git-based binary detection (slower but more thorough)"
        )]
        thorough: bool,
    },
    #[command(about = "Watch for changes and auto-index")]
    Daemon {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long, default_value = "30")]
        interval: u64,
        #[arg(long)]
        no_ignore: bool,
        #[arg(
            long,
            help = "Include uncommitted working tree changes and untracked files"
        )]
        dirty: bool,
        #[arg(
            long,
            help = "Use git-based binary detection (slower but more thorough)"
        )]
        thorough: bool,
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
    Status {
        /// Path to a repository or subdirectory to check (omit to list all filesets)
        path: Option<PathBuf>,
    },
    #[command(about = "Delete a fileset")]
    Delete { fileset: String },
    #[command(about = "Show authentication status")]
    Auth,
    #[command(about = "Show which files would be indexed and their filter status")]
    Files {
        #[arg(default_values_t = vec![String::from(".")])]
        paths: Vec<String>,
        #[arg(long, help = "Index files even if they match .gitignore rules")]
        no_ignore: bool,
        #[arg(
            long,
            help = "Include uncommitted working tree changes and untracked files"
        )]
        dirty: bool,
        #[arg(
            long,
            help = "Use git-based binary detection (slower but more thorough)"
        )]
        thorough: bool,
        #[arg(long, help = "Display as tree instead of flat list")]
        tree: bool,
        #[arg(long, help = "Max directory depth for tree display")]
        depth: Option<usize>,
        #[arg(long, help = "Show .gitignore'd files in output")]
        include_ignored: bool,
        #[arg(long, help = "Disable collapsing fully-skipped directories")]
        no_compact: bool,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let format = cli.output_format();

    if format == OutputFormat::Plain {
        init_tracing();
    }

    if let Err(e) = run(cli, format).await {
        output::print_error(&e, format);
        std::process::exit(1);
    }
}

async fn run(cli: Cli, format: OutputFormat) -> Result<()> {
    let interactive = format == OutputFormat::Plain;

    match cli.command {
        Commands::Index {
            path,
            since,
            no_ignore,
            dirty,
            thorough,
        } => {
            let token = get_token(interactive).await?;
            let result = run_index_core(token, path, since, no_ignore, dirty, thorough).await?;
            output::print_index(&result, format);
        }
        Commands::Daemon {
            path,
            interval,
            no_ignore,
            dirty,
            thorough,
        } => {
            daemon::run_daemon(path, interval, no_ignore, dirty, thorough, interactive).await?;
        }
        Commands::Search {
            query,
            fileset,
            limit,
        } => {
            let token = get_token(interactive).await?;
            let response = run_search(token, query, fileset, limit).await?;
            output::print_search(&response, format);
        }
        Commands::Status { path } => {
            let token = get_token(interactive).await?;
            let response = run_status(token.clone()).await?;
            match path {
                Some(p) => {
                    let repo_root = diff::get_repo_root(&p)?;
                    let fileset_name = repo_root
                        .file_name()
                        .and_then(|name| name.to_str())
                        .map(ToOwned::to_owned)
                        .ok_or_else(|| {
                            anyhow!("failed to derive fileset name from {}", repo_root.display())
                        })?;
                    let fileset = response
                        .filesets
                        .iter()
                        .find(|f| f.name == fileset_name)
                        .cloned();
                    let local_state = state::StateFile::load();
                    let local = local_state.get(&fileset_name);
                    output::print_project_status(
                        &repo_root,
                        &fileset_name,
                        fileset.as_ref(),
                        local,
                        format,
                    );
                }
                None => {
                    output::print_status(&response, format);
                }
            }
        }
        Commands::Delete { fileset } => {
            let token = get_token(interactive).await?;
            run_delete(token, &fileset).await?;
            let mut local_state = state::StateFile::load();
            local_state.remove(&fileset);
            let _ = local_state.save();
            output::print_delete(&fileset, format);
        }
        Commands::Auth => {
            let info = run_auth(interactive).await?;
            output::print_auth(&info, format);
        }
        Commands::Files {
            paths,
            no_ignore,
            dirty,
            thorough,
            tree,
            depth,
            include_ignored,
            no_compact,
        } => {
            let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
            files::run_files(paths, no_ignore, dirty, thorough, tree, depth, include_ignored, !no_compact, format)?;
        }
    }
    Ok(())
}

fn init_tracing() {
    let filter = match EnvFilter::try_from_default_env() {
        Ok(value) => value,
        Err(_) => EnvFilter::new("info"),
    };

    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(filter)
        .try_init();
}

struct FileData {
    relative_path: String,
    content: Vec<u8>,
    doc_sha: [u8; 20],
}

fn collect_file_data(
    repo_root: &Path,
    files: Vec<PathBuf>,
    ignored: &HashSet<String>,
    git_binaries: &HashSet<String>,
) -> Vec<FileData> {
    let mut result = Vec::new();

    for relative in files {
        let normalized = relative.to_string_lossy().replace('\\', "/");
        if ignored.contains(&normalized) {
            tracing::debug!(path = %normalized, "skipped (gitignored)");
            continue;
        }

        if git_binaries.contains(&normalized) {
            tracing::debug!(path = %normalized, "skipped (git binary)");
            continue;
        }

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

pub struct IndexResult {
    pub fileset_name: String,
    pub api_checkpoint: String,
    pub head: String,
    pub files_indexed: usize,
    pub files_uploaded: usize,
    pub elapsed: std::time::Duration,
    pub skipped: bool,
}

pub async fn run_index_core(
    token: String,
    path: PathBuf,
    since: Option<String>,
    no_ignore: bool,
    dirty: bool,
    thorough: bool,
) -> Result<IndexResult> {
    let b64 = base64::engine::general_purpose::STANDARD;
    let started = Instant::now();
    let client = IngestClient::new(token)?;

    let repo_root = diff::get_repo_root(&path)?;
    let fileset_name = repo_root
        .file_name()
        .and_then(|name| name.to_str())
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow!("failed to derive fileset name from {}", repo_root.display()))?;
    let git_head = diff::get_current_head(&repo_root)?;

    let dirty_files = if dirty {
        diff::get_dirty_files(&repo_root)?
    } else {
        Vec::new()
    };

    let mut local_state = state::StateFile::load();

    if since.is_none()
        && dirty_files.is_empty()
        && let Some(saved) = local_state.get(&fileset_name)
        && saved.head == git_head
    {
        info!(fileset = %fileset_name, head = %git_head, "HEAD unchanged, skipping");
        return Ok(IndexResult {
            fileset_name,
            api_checkpoint: saved.checkpoint.clone(),
            head: git_head,
            files_indexed: 0,
            files_uploaded: 0,
            elapsed: started.elapsed(),
            skipped: true,
        });
    }

    let effective_since = since.or_else(|| local_state.get(&fileset_name).map(|s| s.head.clone()));

    let (mut candidates, has_deletions) = match effective_since.as_deref() {
        Some(previous) => {
            let deleted = diff::get_deleted_files(&repo_root, previous, &git_head)?;
            let has_deletions = !deleted.is_empty();
            if has_deletions {
                info!(deleted = deleted.len(), "detected deleted files");
            }
            let changed = diff::get_changed_files(&repo_root, previous, &git_head)?;
            (changed, has_deletions)
        }
        None => (diff::get_all_tracked_files(&repo_root)?, false),
    };

    if !dirty_files.is_empty() {
        let existing: HashSet<PathBuf> = candidates.iter().cloned().collect();
        for f in dirty_files {
            if !existing.contains(&f) {
                candidates.push(f);
            }
        }
        info!(files = candidates.len(), "merged dirty working tree files");
    }

    let ignored = if no_ignore {
        HashSet::new()
    } else {
        diff::check_ignored(&repo_root, &candidates)?
    };

    let git_binaries = if thorough {
        info!("running git-based binary detection");
        diff::get_git_binary_files(&repo_root)?
    } else {
        HashSet::new()
    };

    let file_data = collect_file_data(&repo_root, candidates, &ignored, &git_binaries);
    let files_indexed = file_data.len();
    info!(files = files_indexed, "collected files for indexing");

    if file_data.is_empty() && !has_deletions {
        info!("no files to index");
        local_state.update(
            &fileset_name,
            &git_head,
            "",
            &repo_root.to_string_lossy(),
            0,
        );
        let _ = local_state.save();
        return Ok(IndexResult {
            fileset_name,
            api_checkpoint: String::new(),
            head: git_head,
            files_indexed: 0,
            files_uploaded: 0,
            elapsed: started.elapsed(),
            skipped: false,
        });
    }

    let doc_shas: Vec<[u8; 20]> = file_data.iter().map(|f| f.doc_sha).collect();

    let mut checkpoint_hasher = Sha1::new();
    for sha in &doc_shas {
        checkpoint_hasher.update(sha);
    }
    let new_checkpoint = b64.encode(checkpoint_hasher.finalize());

    info!("initializing blackbird WASM runtime");
    let mut bb = BlackbirdWasm::new()?;

    for (i, sha) in doc_shas.iter().enumerate() {
        tracing::debug!(idx = i, sha = %hex::encode(sha), "doc_sha");
    }

    let geo_filter_bytes = bb.compute_geo_filter(&doc_shas)?;
    let geo_filter = b64.encode(&geo_filter_bytes);
    tracing::debug!(hex = %hex::encode(&geo_filter_bytes), "geo_filter raw");
    info!(bytes = geo_filter_bytes.len(), "computed geo_filter");

    let initial_symbols = bb.create_coded_symbols(&doc_shas, 0, 1)?;
    let coded_symbols: Vec<String> = initial_symbols.iter().map(|s| b64.encode(s)).collect();
    for (i, s) in initial_symbols.iter().enumerate() {
        tracing::debug!(idx = i, hex = %hex::encode(s), "coded_symbol");
    }
    info!(
        count = coded_symbols.len(),
        "computed initial coded_symbols"
    );

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
        info!(fileset = %fileset_name, checkpoint = %new_checkpoint, "already indexed");
        local_state.update(
            &fileset_name,
            &git_head,
            &new_checkpoint,
            &repo_root.to_string_lossy(),
            files_indexed,
        );
        let _ = local_state.save();
        return Ok(IndexResult {
            fileset_name,
            api_checkpoint: new_checkpoint,
            head: git_head,
            files_indexed,
            files_uploaded: 0,
            elapsed: started.elapsed(),
            skipped: false,
        });
    }

    let ingest_id = ingest.ingest_id;
    info!(ingest_id = %ingest_id, "ingest created");

    let mut next_range = Some(ingest.coded_symbol_range);
    while let Some(range) = next_range.take() {
        if range.start >= range.end {
            break;
        }
        let symbols = bb.create_coded_symbols(&doc_shas, range.start as u32, range.end as u32)?;
        let encoded: Vec<String> = symbols.iter().map(|s| b64.encode(s)).collect();
        let total = encoded.len();

        const CHUNK_SIZE: usize = 5000;
        let chunks: Vec<&[String]> = encoded.chunks(CHUNK_SIZE).collect();
        let num_chunks = chunks.len();

        let mut last_resp = None;
        for (i, chunk) in chunks.into_iter().enumerate() {
            let chunk_start = range.start + (i * CHUNK_SIZE) as u64;
            let chunk_end = chunk_start + chunk.len() as u64;
            info!(
                chunk = i + 1,
                total_chunks = num_chunks,
                start = chunk_start,
                end = chunk_end,
                count = chunk.len(),
                total_symbols = total,
                "uploading coded_symbols"
            );

            last_resp = Some(
                client
                    .upload_symbols(crate::types::UploadSymbolsRequest {
                        ingest_id: ingest_id.clone(),
                        coded_symbols: chunk.to_vec(),
                        coded_symbol_range: crate::types::Range {
                            start: chunk_start,
                            end: chunk_end,
                        },
                    })
                    .await?,
            );
        }

        next_range = last_resp.and_then(|r| r.next_coded_symbol_range);
    }

    let doc_map: std::collections::HashMap<String, &FileData> = file_data
        .iter()
        .map(|f| (b64.encode(f.doc_sha), f))
        .collect();

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

    client.finalize(FinalizeRequest { ingest_id }).await?;

    let elapsed = started.elapsed();
    info!(
        files = files_indexed,
        uploaded,
        ?elapsed,
        fileset = %fileset_name,
        checkpoint = &new_checkpoint[..new_checkpoint.len().min(14)],
        "indexing complete"
    );

    local_state.update(
        &fileset_name,
        &git_head,
        &new_checkpoint,
        &repo_root.to_string_lossy(),
        files_indexed,
    );
    let _ = local_state.save();

    Ok(IndexResult {
        fileset_name,
        api_checkpoint: new_checkpoint,
        head: git_head,
        files_indexed,
        files_uploaded: uploaded,
        elapsed,
        skipped: false,
    })
}

fn relative_to_api_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

async fn run_search(
    token: String,
    query: String,
    fileset: String,
    limit: u32,
) -> Result<types::SearchResponse> {
    let client = IngestClient::new(token)?;

    client
        .search(SearchRequest {
            prompt: query,
            scoping_query: format!("fileset:{fileset}"),
            embedding_model: "metis-1024-I16-Binary".to_string(),
            limit,
        })
        .await
}

async fn run_status(token: String) -> Result<types::ListFilesetsResponse> {
    let client = IngestClient::new(token)?;
    client.list_filesets().await
}

async fn run_delete(token: String, fileset: &str) -> Result<()> {
    let client = IngestClient::new(token)?;
    client
        .delete_fileset(DeleteFilesetRequest {
            fileset_name: fileset.to_string(),
        })
        .await
}
