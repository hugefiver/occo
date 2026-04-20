mod auth;
mod client;
mod daemon;
mod diff;
mod files;
mod filter;
mod output;
mod state;
mod types;
mod vscode_local;
mod wasm;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Result, anyhow, bail};
use base64::Engine as _;
use clap::{Parser, Subcommand};
use sha1::{Digest, Sha1};
use indicatif::{ProgressBar, ProgressStyle};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use crate::auth::{get_token, run_auth};
use crate::client::IngestClient;
use crate::output::OutputFormat;
use crate::types::{
    CreateIngestRequest, DeleteFilesetRequest, FinalizeRequest, SearchRequest, SearchSource,
    TaggedSearchResult, UploadDocumentRequest,
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
        #[arg(long, help = "Disable progress bar output")]
        no_progress: bool,
        #[arg(long, help = "Trigger GitHub remote semantic indexing after local index")]
        auto_github_index: bool,
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
        #[arg(long, help = "Disable progress bar output")]
        no_progress: bool,
    },
    #[command(about = "Search the semantic index")]
    Search {
        query: String,
        #[arg(long)]
        fileset: Option<String>,
        #[arg(long)]
        repo: Option<String>,
        #[arg(long, default_value = "10")]
        limit: u32,
        #[arg(long)]
        no_github: bool,
        #[arg(long)]
        no_external_ingest: bool,
        #[arg(long, conflicts_with = "external_ingest_only")]
        github_only: bool,
        #[arg(long, conflicts_with = "github_only")]
        external_ingest_only: bool,
        #[arg(long)]
        auto_github_index: bool,
        #[arg(long)]
        vscode_local: bool,
    },
    #[command(about = "List indexed filesets and their status")]
    Status {
        /// Path to a repository or subdirectory to check (omit to list all filesets)
        path: Option<PathBuf>,
    },
    #[command(about = "Delete a fileset")]
    Delete { fileset: String },
    #[command(about = "Show authentication status")]
    Auth {
        #[arg(long, help = "Force re-authentication even if a token already exists")]
        force: bool,
    },
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
            no_progress,
            auto_github_index,
        } => {
            let token = get_token(interactive).await?;
            let progress = interactive && !no_progress;

            let effective_since = if !auto_github_index {
                since
            } else {
                let repo_root = diff::get_repo_root(&path).ok();
                let nwo = repo_root.as_ref().and_then(|root| {
                    diff::get_github_remote(root).ok().flatten()
                });
                if let Some((owner, name)) = &nwo {
                    let repo = format!("{owner}/{name}");
                    let client = IngestClient::new(token.clone())?;
                    match client.check_github_index(&repo).await {
                        Ok(status) if status.semantic_code_search_ok => {
                            if let Some(sha) = &status.semantic_commit_sha {
                                info!(
                                    repo = %repo,
                                    remote_sha = %sha,
                                    "GitHub remote index covers up to this commit, only indexing local delta"
                                );
                                since.or_else(|| Some(sha.clone()))
                            } else {
                                info!(repo = %repo, "GitHub remote index active but no commit SHA, doing full index");
                                since
                            }
                        }
                        _ => {
                            info!("GitHub remote index not available, doing full index");
                            since
                        }
                    }
                } else {
                    since
                }
            };

            let result = run_index_core(token.clone(), path.clone(), effective_since, no_ignore, dirty, thorough, progress).await?;
            output::print_index(&result, format);

            if auto_github_index {
                let repo_root = diff::get_repo_root(&path).ok();
                let nwo = repo_root.as_ref().and_then(|root| {
                    diff::get_github_remote(root).ok().flatten()
                });
                if let Some((owner, name)) = nwo {
                    let repo = format!("{owner}/{name}");
                    let client = IngestClient::new(token)?;
                    match client.check_github_index(&repo).await {
                        Ok(status) if status.semantic_indexing_enabled && status.semantic_code_search_ok => {
                            info!(repo = %repo, "GitHub remote index already active");
                        }
                        _ => {
                            info!(repo = %repo, "triggering GitHub remote indexing");
                            match client.trigger_github_indexing(&repo).await {
                                Ok(()) => info!(repo = %repo, "GitHub remote indexing triggered"),
                                Err(e) => {
                                    warn!(repo = %repo, error = %e, "trigger failed with Copilot token, trying VS Code auth");
                                    let vscode_token = match auth::read_vscode_token() {
                                        Ok(Some(t)) => Some(t),
                                        _ => {
                                            match auth::vscode_device_flow_token("repo").await {
                                                Ok(t) => {
                                                    if let Err(save_err) = auth::save_vscode_token(&t) {
                                                        warn!(error = %save_err, "failed to save VS Code token");
                                                    }
                                                    Some(t)
                                                }
                                                Err(auth_err) => {
                                                    warn!(error = %auth_err, "VS Code authentication failed");
                                                    None
                                                }
                                            }
                                        }
                                    };
                                    if let Some(token) = vscode_token {
                                        match IngestClient::new(token)?.trigger_github_indexing(&repo).await {
                                            Ok(()) => info!(repo = %repo, "GitHub remote indexing triggered with VS Code token"),
                                            Err(e2) => warn!(repo = %repo, error = %e2, "still could not trigger remote indexing"),
                                        }
                                    }
                                }
                            }
                        }
                    }
                } else {
                    warn!("no GitHub remote detected, skipping remote index trigger");
                }
            }
        }
        Commands::Daemon {
            path,
            interval,
            no_ignore,
            dirty,
            thorough,
            no_progress,
        } => {
            let progress = interactive && !no_progress;
            daemon::run_daemon(path, interval, no_ignore, dirty, thorough, progress, interactive).await?;
        }
        Commands::Search {
            query,
            fileset,
            repo,
            limit,
            no_github,
            no_external_ingest,
            github_only,
            external_ingest_only,
            auto_github_index,
            vscode_local,
        } => {
            let token = get_token(interactive).await?;
            let use_github = !no_github && !external_ingest_only;
            let use_external_ingest = !no_external_ingest && !github_only;
            let use_vscode_local = vscode_local;
            let response = run_search(
                token,
                query,
                fileset,
                repo,
                limit,
                use_github,
                use_external_ingest,
                use_vscode_local,
                auto_github_index,
            )
            .await?;
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
                    let github_info = match diff::get_github_remote(&repo_root) {
                        Ok(Some((owner, name))) => {
                            let nwo = format!("{owner}/{name}");
                            let client = IngestClient::new(token)?;
                            let indexed = client.check_github_index(&nwo).await.ok();
                            Some((nwo, indexed))
                        }
                        _ => None,
                    };
                    output::print_project_status(
                        &repo_root,
                        &fileset_name,
                        fileset.as_ref(),
                        local,
                        github_info.as_ref(),
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
        Commands::Auth { force } => {
            let info = run_auth(interactive, force).await?;
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

fn make_progress_bar(len: u64, msg: &str) -> ProgressBar {
    let pb = ProgressBar::new(len);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("  {spinner:.green} {msg} [{bar:30.cyan/blue}] {pos}/{len}")
            .unwrap()
            .progress_chars("=> "),
    );
    pb.set_message(msg.to_string());
    pb
}

fn make_spinner(msg: &str) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .template("  {spinner:.green} {msg}")
            .unwrap(),
    );
    pb.set_message(msg.to_string());
    pb.enable_steady_tick(std::time::Duration::from_millis(100));
    pb
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
    pb: Option<&ProgressBar>,
) -> Vec<FileData> {
    let mut result = Vec::new();

    for relative in files {
        let normalized = relative.to_string_lossy().replace('\\', "/");
        if ignored.contains(&normalized) {
            tracing::debug!(path = %normalized, "skipped (gitignored)");
            if let Some(pb) = pb { pb.inc(1); }
            continue;
        }

        if git_binaries.contains(&normalized) {
            tracing::debug!(path = %normalized, "skipped (git binary)");
            if let Some(pb) = pb { pb.inc(1); }
            continue;
        }

        let absolute = repo_root.join(&relative);

        let metadata = match std::fs::metadata(&absolute) {
            Ok(meta) => meta,
            Err(error) => {
                warn!(path = %absolute.display(), %error, "skipping file: metadata read failed");
                if let Some(pb) = pb { pb.inc(1); }
                continue;
            }
        };

        if !metadata.is_file() {
            if let Some(pb) = pb { pb.inc(1); }
            continue;
        }

        if !filter::can_ingest(&relative, metadata.len()) {
            if let Some(pb) = pb { pb.inc(1); }
            continue;
        }

        let bytes = match std::fs::read(&absolute) {
            Ok(content) => content,
            Err(error) => {
                warn!(path = %absolute.display(), %error, "skipping file: content read failed");
                if let Some(pb) = pb { pb.inc(1); }
                continue;
            }
        };

        if !filter::can_ingest_content(&bytes) {
            if let Some(pb) = pb { pb.inc(1); }
            continue;
        }

        let rel_path = relative_to_api_path(&relative);
        let doc_sha = BlackbirdWasm::get_doc_sha(&rel_path, &bytes);

        result.push(FileData {
            relative_path: rel_path,
            content: bytes,
            doc_sha,
        });
        if let Some(pb) = pb { pb.inc(1); }
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
    progress: bool,
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

    let pb_collect = if progress {
        Some(make_progress_bar(candidates.len() as u64, "Collecting files"))
    } else {
        None
    };
    let file_data = collect_file_data(&repo_root, candidates, &ignored, &git_binaries, pb_collect.as_ref());
    if let Some(pb) = pb_collect { pb.finish_and_clear(); }
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

    let sp_wasm = if progress { Some(make_spinner("Initializing WASM runtime")) } else { None };
    info!("initializing blackbird WASM runtime");
    let mut bb = BlackbirdWasm::new()?;
    if let Some(sp) = sp_wasm { sp.finish_and_clear(); }

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

    let sp_symbols = if progress { Some(make_spinner("Uploading symbols")) } else { None };
    let mut next_range = Some(ingest.coded_symbol_range);
    while let Some(range) = next_range.take() {
        if range.start >= range.end {
            break;
        }
        let symbols = bb.create_coded_symbols(&doc_shas, range.start as u32, range.end as u32)?;
        let encoded: Vec<String> = symbols.iter().map(|s| b64.encode(s)).collect();
        if let Some(sp) = &sp_symbols {
            sp.set_message(format!("Uploading symbols ({}/{})", range.start, range.end));
        }
        info!(
            start = range.start,
            end = range.end,
            count = encoded.len(),
            "uploading coded_symbols"
        );

        let resp = client
            .upload_symbols(crate::types::UploadSymbolsRequest {
                ingest_id: ingest_id.clone(),
                coded_symbols: encoded,
                coded_symbol_range: range,
            })
            .await?;

        next_range = resp.next_coded_symbol_range;
    }
    if let Some(sp) = sp_symbols { sp.finish_and_clear(); }

    let doc_map: std::collections::HashMap<String, &FileData> = file_data
        .iter()
        .map(|f| (b64.encode(f.doc_sha), f))
        .collect();

    let pb_upload = if progress {
        Some(make_progress_bar(files_indexed as u64, "Uploading documents"))
    } else {
        None
    };
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
        client.upload_documents_concurrent(uploads, pb_upload.as_ref()).await?;

        match batch.next_page_token {
            Some(token) if !token.is_empty() => page_token = token,
            _ => break,
        }
    }
    if let Some(pb) = pb_upload { pb.finish_and_clear(); }
    info!(uploaded, "documents uploaded");

    let sp_finalize = if progress { Some(make_spinner("Finalizing")) } else { None };
    client.finalize(FinalizeRequest { ingest_id }).await?;
    if let Some(sp) = sp_finalize { sp.finish_and_clear(); }

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

const EMBEDDING_MODEL: &str = "metis-1024-I16-Binary";

#[allow(clippy::too_many_arguments)]
async fn run_search(
    token: String,
    query: String,
    fileset: Option<String>,
    repo: Option<String>,
    limit: u32,
    use_github: bool,
    use_external_ingest: bool,
    use_vscode_local: bool,
    auto_github_index: bool,
) -> Result<types::HybridSearchResponse> {
    let client = IngestClient::new(token)?;

    let repo_nwo = if use_github {
        match repo {
            Some(r) => Some(r),
            None => {
                let cwd = std::env::current_dir()?;
                diff::get_repo_root(&cwd).ok().and_then(|root| {
                    diff::get_github_remote(&root)
                        .ok()
                        .flatten()
                        .map(|(o, n)| format!("{o}/{n}"))
                })
            }
        }
    } else {
        None
    };

    let fileset_name = if use_external_ingest {
        match fileset {
            Some(f) => Some(f),
            None => {
                let cwd = std::env::current_dir()?;
                diff::get_repo_root(&cwd).ok().and_then(|root| {
                    root.file_name()
                        .and_then(|n| n.to_str())
                        .map(String::from)
                })
            }
        }
    } else {
        None
    };

    if repo_nwo.is_none() && fileset_name.is_none() && !use_vscode_local {
        bail!("no search sources available: could not detect GitHub remote or fileset name");
    }

    if auto_github_index
        && let Some(nwo) = &repo_nwo {
            match client.check_github_index(nwo).await {
                Ok(status) if !status.semantic_indexing_enabled => {
                    info!(repo = %nwo, "triggering GitHub semantic indexing");
                    if let Err(e) = client.trigger_github_indexing(nwo).await {
                        warn!(repo = %nwo, error = %e, "failed to trigger indexing");
                    }
                }
                _ => {}
            }
        }

    // Parallel search
    let github_fut = async {
        match &repo_nwo {
            Some(nwo) => {
                let resp = client
                    .search_github(SearchRequest {
                        prompt: query.clone(),
                        scoping_query: format!("repo:{nwo}"),
                        embedding_model: EMBEDDING_MODEL.to_string(),
                        limit,
                        include_embeddings: Some(false),
                    })
                    .await;
                Some((nwo.clone(), resp))
            }
            None => None,
        }
    };

    let ingest_fut = async {
        match &fileset_name {
            Some(fs) => {
                let resp = client
                    .search(SearchRequest {
                        prompt: query.clone(),
                        scoping_query: format!("fileset:{fs}"),
                        embedding_model: EMBEDDING_MODEL.to_string(),
                        limit,
                        include_embeddings: Some(false),
                    })
                    .await;
                Some((fs.clone(), resp))
            }
            None => None,
        }
    };

    let (github_result, ingest_result) = tokio::join!(github_fut, ingest_fut);

    let mut all_results: Vec<TaggedSearchResult> = Vec::new();
    let mut embedding_model = EMBEDDING_MODEL.to_string();

    if let Some((nwo, result)) = github_result {
        match result {
            Ok(resp) => {
                embedding_model.clone_from(&resp.embedding_model);
                for r in resp.results {
                    all_results.push(TaggedSearchResult {
                        source: SearchSource::GitHub(nwo.clone()),
                        result: r,
                    });
                }
            }
            Err(e) => warn!(source = "github", error = %e, "search failed"),
        }
    }

    if let Some((fs, result)) = ingest_result {
        match result {
            Ok(resp) => {
                embedding_model.clone_from(&resp.embedding_model);
                for r in resp.results {
                    all_results.push(TaggedSearchResult {
                        source: SearchSource::ExternalIngest(fs.clone()),
                        result: r,
                    });
                }
            }
            Err(e) => warn!(source = "external_ingest", error = %e, "search failed"),
        }
    }

    if use_vscode_local {
        let cwd = std::env::current_dir()?;
        if let Ok(root) = diff::get_repo_root(&cwd) {
            match vscode_local::search_vscode_local(&root, &query, limit) {
                Ok(results) => {
                    for r in results {
                        all_results.push(TaggedSearchResult {
                            source: SearchSource::VscodeLocal,
                            result: r,
                        });
                    }
                }
                Err(e) => warn!(source = "vscode_local", error = %e, "search failed"),
            }
        }
    }

    all_results = deduplicate_results(all_results);
    all_results.sort_by(|a, b| {
        a.result
            .distance
            .partial_cmp(&b.result.distance)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    all_results.truncate(limit as usize);

    Ok(types::HybridSearchResponse {
        results: all_results,
        embedding_model,
    })
}

fn deduplicate_results(results: Vec<TaggedSearchResult>) -> Vec<TaggedSearchResult> {
    let mut seen: HashMap<(String, String), usize> = HashMap::new();
    let mut deduped: Vec<TaggedSearchResult> = Vec::new();

    for r in results {
        let key = (r.result.location.path.clone(), r.result.chunk.hash.clone());
        if let Some(&idx) = seen.get(&key) {
            if r.result.distance < deduped[idx].result.distance {
                deduped[idx] = r;
            }
        } else {
            seen.insert(key, deduped.len());
            deduped.push(r);
        }
    }
    deduped
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
