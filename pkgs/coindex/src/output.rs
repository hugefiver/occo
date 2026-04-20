use std::path::Path;

use crate::auth::AuthInfo;
use crate::state::FilesetState;
use crate::types::{Fileset, HybridSearchResponse, IndexStatusResponse, ListFilesetsResponse};
use crate::IndexResult;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OutputFormat {
    Plain,
    Json,
    Markdown,
}

pub fn print_error(error: &anyhow::Error, format: OutputFormat) {
    match format {
        OutputFormat::Plain => {
            eprintln!("Error: {error:#}");
        }
        OutputFormat::Json => {
            println!("{}", serde_json::json!({ "error": format!("{error:#}") }));
        }
        OutputFormat::Markdown => {
            println!("**Error**: {error:#}");
        }
    }
}

pub fn print_index(result: &IndexResult, format: OutputFormat) {
    match format {
        OutputFormat::Plain => {
            if result.skipped {
                println!(
                    "Fileset '{}' already up to date (HEAD {})",
                    result.fileset_name,
                    &result.head[..result.head.len().min(12)]
                );
            } else {
                println!(
                    "Indexed {} files ({} uploaded) for fileset '{}' in {:.1}s",
                    result.files_indexed,
                    result.files_uploaded,
                    result.fileset_name,
                    result.elapsed.as_secs_f64(),
                );
            }
        }
        OutputFormat::Json => {
            println!(
                "{}",
                serde_json::json!({
                    "fileset_name": result.fileset_name,
                    "head": result.head,
                    "api_checkpoint": result.api_checkpoint,
                    "files_indexed": result.files_indexed,
                    "files_uploaded": result.files_uploaded,
                    "elapsed_secs": result.elapsed.as_secs_f64(),
                    "skipped": result.skipped,
                })
            );
        }
        OutputFormat::Markdown => {
            if result.skipped {
                println!("## Index Skipped\n");
                println!(
                    "Fileset **{}** already up to date at `{}`",
                    result.fileset_name,
                    &result.head[..result.head.len().min(12)]
                );
            } else {
                println!("## Index Complete\n");
                println!("- **Fileset**: {}", result.fileset_name);
                println!(
                    "- **HEAD**: `{}`",
                    &result.head[..result.head.len().min(12)]
                );
                println!("- **Files indexed**: {}", result.files_indexed);
                println!("- **Files uploaded**: {}", result.files_uploaded);
                println!("- **Elapsed**: {:.1}s", result.elapsed.as_secs_f64());
            }
        }
    }
}

pub fn print_search(response: &HybridSearchResponse, format: OutputFormat) {
    match format {
        OutputFormat::Plain => {
            if response.results.is_empty() {
                println!("No results found.");
                return;
            }
            for (i, r) in response.results.iter().enumerate() {
                let lang = r
                    .result
                    .location
                    .language
                    .as_ref()
                    .map(|l| l.name.as_str())
                    .unwrap_or("?");
                let lines = r
                    .result
                    .chunk
                    .line_range
                    .as_ref()
                    .map(|lr| format!(" L{}-{}", lr.start, lr.end))
                    .unwrap_or_default();
                println!(
                    "[{}] [{}] {} ({}) dist={:.4}{}",
                    i + 1,
                    r.source.label(),
                    r.result.location.path,
                    lang,
                    r.result.distance,
                    lines,
                );
                let text = r.result.chunk.text.replace('\n', "\n    ");
                if text.len() > 500 {
                    let end = text
                        .char_indices()
                        .map(|(i, _)| i)
                        .find(|&i| i >= 500)
                        .unwrap_or(text.len());
                    println!("    {}...\n", &text[..end]);
                } else {
                    println!("    {}\n", text);
                }
            }
        }
        OutputFormat::Json => {
            let items: Vec<serde_json::Value> = response
                .results
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "source": { "type": r.source.name(), "label": r.source.label() },
                        "path": r.result.location.path,
                        "language": r.result.location.language.as_ref().map(|l| &l.name),
                        "distance": r.result.distance,
                        "line_range": r.result.chunk.line_range.as_ref().map(|lr| {
                            serde_json::json!({ "start": lr.start, "end": lr.end })
                        }),
                        "text": r.result.chunk.text,
                    })
                })
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "embedding_model": response.embedding_model,
                    "results": items,
                }))
                .unwrap_or_default()
            );
        }
        OutputFormat::Markdown => {
            if response.results.is_empty() {
                println!("No results found.");
                return;
            }
            for (i, r) in response.results.iter().enumerate() {
                let lang = r
                    .result
                    .location
                    .language
                    .as_ref()
                    .map(|l| l.name.as_str())
                    .unwrap_or("unknown");
                let lines = r
                    .result
                    .chunk
                    .line_range
                    .as_ref()
                    .map(|lr| format!("L{}-{}", lr.start, lr.end))
                    .unwrap_or_default();
                println!(
                    "### {}. `{}` ({}) [{}]",
                    i + 1,
                    r.result.location.path,
                    lang,
                    r.source.label()
                );
                if !lines.is_empty() {
                    println!("{} · distance: {:.4}\n", lines, r.result.distance);
                } else {
                    println!("distance: {:.4}\n", r.result.distance);
                }
                let text = r.result.chunk.text.as_str();
                println!("```{}\n{}\n```\n", lang.to_lowercase(), text);
            }
        }
    }
}

pub fn print_status(response: &ListFilesetsResponse, format: OutputFormat) {
    match format {
        OutputFormat::Plain => {
            if response.filesets.is_empty() {
                println!("No filesets.");
                return;
            }
            for f in &response.filesets {
                let cp = if f.checkpoint.len() > 14 {
                    &f.checkpoint[..14]
                } else {
                    &f.checkpoint
                };
                println!("  {}  status={}  checkpoint={}", f.name, f.status, cp);
            }
            println!(
                "Max filesets: {} (used: {})",
                response.max_filesets,
                response.filesets.len()
            );
        }
        OutputFormat::Json => {
            println!(
                "{}",
                serde_json::to_string_pretty(response).unwrap_or_default()
            );
        }
        OutputFormat::Markdown => {
            if response.filesets.is_empty() {
                println!("No filesets.");
                return;
            }
            println!("## Filesets\n");
            println!("| Name | Status | Checkpoint |");
            println!("|------|--------|------------|");
            for f in &response.filesets {
                let cp = if f.checkpoint.len() > 14 {
                    &f.checkpoint[..14]
                } else {
                    &f.checkpoint
                };
                println!("| {} | {} | {} |", f.name, f.status, cp);
            }
            println!("\nMax filesets: {}", response.max_filesets);
        }
    }
}

pub fn print_delete(fileset: &str, format: OutputFormat) {
    match format {
        OutputFormat::Plain => {
            println!("Deleted fileset '{fileset}'.");
        }
        OutputFormat::Json => {
            println!(
                "{}",
                serde_json::json!({
                    "deleted": true,
                    "fileset_name": fileset,
                })
            );
        }
        OutputFormat::Markdown => {
            println!("Deleted fileset `{fileset}`.");
        }
    }
}

pub fn print_project_status(
    repo_root: &Path,
    fileset_name: &str,
    fileset: Option<&Fileset>,
    local: Option<&FilesetState>,
    github: Option<&(String, Option<IndexStatusResponse>)>,
    format: OutputFormat,
) {
    match format {
        OutputFormat::Plain => {
            match fileset {
                Some(f) => {
                    println!(
                        "Indexed: {}  status={}  checkpoint={}",
                        f.name,
                        f.status,
                        &f.checkpoint[..f.checkpoint.len().min(14)]
                    );
                }
                None => {
                    println!("Not indexed (expected fileset '{fileset_name}').");
                }
            }
            if let Some(s) = local {
                println!(
                    "Local state: head={}  files={}  last_indexed={}",
                    &s.head[..s.head.len().min(12)],
                    s.files_indexed,
                    s.indexed_at,
                );
            }
            if let Some((nwo, status)) = github {
                match status {
                    Some(s) => {
                        let semantic = if s.semantic_code_search_ok {
                            "ready"
                        } else {
                            "not available"
                        };
                        let commit = s.semantic_commit_sha.as_deref().unwrap_or("none");
                        println!(
                            "GitHub: {}  semantic={}  commit={}",
                            nwo,
                            semantic,
                            &commit[..commit.len().min(12)]
                        );
                    }
                    None => println!("GitHub: {}  (could not check status)", nwo),
                }
            }
        }
        OutputFormat::Json => {
            let mut obj = serde_json::json!({
                "indexed": fileset.is_some(),
                "fileset_name": fileset_name,
                "repo_root": repo_root.to_string_lossy(),
            });
            if let Some(f) = fileset {
                obj["status"] = serde_json::Value::String(f.status.clone());
                obj["checkpoint"] = serde_json::Value::String(f.checkpoint.clone());
            }
            if let Some(s) = local {
                obj["local_head"] = serde_json::Value::String(s.head.clone());
                obj["local_checkpoint"] = serde_json::Value::String(s.checkpoint.clone());
                obj["local_files_indexed"] = serde_json::json!(s.files_indexed);
                obj["local_indexed_at"] = serde_json::json!(s.indexed_at);
            }
            if let Some((nwo, status)) = github {
                obj["github_repo"] = serde_json::Value::String(nwo.clone());
                if let Some(s) = status {
                    obj["github_semantic_ok"] = serde_json::json!(s.semantic_code_search_ok);
                    obj["github_indexing_enabled"] = serde_json::json!(s.semantic_indexing_enabled);
                    if let Some(sha) = &s.semantic_commit_sha {
                        obj["github_semantic_commit"] = serde_json::Value::String(sha.clone());
                    }
                }
            }
            println!("{}", serde_json::to_string_pretty(&obj).unwrap_or_default());
        }
        OutputFormat::Markdown => {
            match fileset {
                Some(f) => {
                    println!("## Project Status\n");
                    println!("- **Fileset**: {}", f.name);
                    println!("- **Status**: {}", f.status);
                    println!(
                        "- **Checkpoint**: `{}`",
                        &f.checkpoint[..f.checkpoint.len().min(14)]
                    );
                    println!("- **Repo root**: `{}`", repo_root.display());
                }
                None => {
                    println!(
                        "Not indexed. Expected fileset `{fileset_name}` (repo root: `{}`).",
                        repo_root.display()
                    );
                }
            }
            if let Some(s) = local {
                println!("\n### Local State\n");
                println!("- **HEAD**: `{}`", &s.head[..s.head.len().min(12)]);
                println!("- **Files indexed**: {}", s.files_indexed);
                println!("- **Last indexed**: {}", s.indexed_at);
            }
            if let Some((nwo, status)) = github {
                println!("\n### GitHub Remote Index\n");
                println!("- **Repository**: {nwo}");
                match status {
                    Some(s) => {
                        let semantic = if s.semantic_code_search_ok {
                            "✅ Ready"
                        } else {
                            "❌ Not available"
                        };
                        println!("- **Semantic search**: {semantic}");
                        if let Some(sha) = &s.semantic_commit_sha {
                            println!("- **Indexed commit**: `{}`", &sha[..sha.len().min(12)]);
                        }
                    }
                    None => println!("- Could not check status"),
                }
            }
        }
    }
}

pub fn print_auth(info: &AuthInfo, format: OutputFormat) {
    match format {
        OutputFormat::Plain => {
            if info.authenticated {
                println!(
                    "Authenticated via {}",
                    info.source.as_deref().unwrap_or("unknown")
                );
            } else {
                println!("Not authenticated.");
            }
        }
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(info).unwrap_or_default());
        }
        OutputFormat::Markdown => {
            if info.authenticated {
                println!(
                    "Authenticated via {}",
                    info.source.as_deref().unwrap_or("unknown")
                );
            } else {
                println!("Not authenticated. Run `coindex auth` to set up.");
            }
        }
    }
}
