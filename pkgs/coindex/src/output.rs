use crate::IndexResult;
use crate::auth::AuthInfo;
use crate::types::{ListFilesetsResponse, SearchResponse};

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
            println!(
                "Indexed {} files ({} uploaded) for fileset '{}' in {:.1}s",
                result.files_indexed,
                result.files_uploaded,
                result.fileset_name,
                result.elapsed.as_secs_f64(),
            );
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
                })
            );
        }
        OutputFormat::Markdown => {
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

pub fn print_search(response: &SearchResponse, format: OutputFormat) {
    match format {
        OutputFormat::Plain => {
            if response.results.is_empty() {
                println!("No results found.");
                return;
            }
            for (i, r) in response.results.iter().enumerate() {
                let lang = r
                    .location
                    .language
                    .as_ref()
                    .map(|l| l.name.as_str())
                    .unwrap_or("?");
                let lines = r
                    .chunk
                    .line_range
                    .as_ref()
                    .map(|lr| format!(" L{}-{}", lr.start, lr.end))
                    .unwrap_or_default();
                println!(
                    "[{}] {} ({}) dist={:.4}{}",
                    i + 1,
                    r.location.path,
                    lang,
                    r.distance,
                    lines,
                );
                let text = r.chunk.text.replace('\n', "\n    ");
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
            println!(
                "{}",
                serde_json::to_string_pretty(response).unwrap_or_default()
            );
        }
        OutputFormat::Markdown => {
            if response.results.is_empty() {
                println!("No results found.");
                return;
            }
            for (i, r) in response.results.iter().enumerate() {
                let lang = r
                    .location
                    .language
                    .as_ref()
                    .map(|l| l.name.as_str())
                    .unwrap_or("unknown");
                let lines = r
                    .chunk
                    .line_range
                    .as_ref()
                    .map(|lr| format!("L{}-{}", lr.start, lr.end))
                    .unwrap_or_default();
                println!("### {}. `{}` ({})", i + 1, r.location.path, lang);
                if !lines.is_empty() {
                    println!("{} · distance: {:.4}\n", lines, r.distance);
                } else {
                    println!("distance: {:.4}\n", r.distance);
                }
                println!("```{}\n{}\n```\n", lang.to_lowercase(), r.chunk.text);
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
