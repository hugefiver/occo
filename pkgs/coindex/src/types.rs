#![allow(dead_code)]

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CreateIngestRequest {
    pub fileset_name: String,
    pub new_checkpoint: String,
    pub geo_filter: String,
    pub coded_symbols: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct UploadSymbolsRequest {
    pub ingest_id: String,
    pub coded_symbols: Vec<String>,
    pub coded_symbol_range: Range,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BatchRequest {
    pub ingest_id: String,
    pub page_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct UploadDocumentRequest {
    pub ingest_id: String,
    pub content: String,
    pub file_path: String,
    pub doc_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct FinalizeRequest {
    pub ingest_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DeleteFilesetRequest {
    pub fileset_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SearchRequest {
    pub prompt: String,
    pub scoping_query: String,
    pub embedding_model: String,
    pub limit: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_embeddings: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CreateIngestResponse {
    pub ingest_id: String,
    pub coded_symbol_range: Range,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct UploadSymbolsResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_coded_symbol_range: Option<Range>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BatchResponse {
    pub doc_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_page_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ListFilesetsResponse {
    pub filesets: Vec<Fileset>,
    pub max_filesets: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SearchResponse {
    pub results: Vec<SearchResult>,
    pub embedding_model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Range {
    pub start: u64,
    pub end: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Fileset {
    pub name: String,
    pub checkpoint: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SearchResult {
    pub location: Location,
    pub distance: f64,
    pub chunk: Chunk,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Location {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fileset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc_id: Option<String>,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<Language>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "commitSha")]
    pub commit_sha: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<RepoInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Language {
    pub id: u32,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ChunkRange {
    pub start: u64,
    pub end: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Chunk {
    pub hash: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_range: Option<ChunkRange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<ChunkRange>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoInfo {
    pub nwo: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchSource {
    ExternalIngest(String),
    GitHub(String),
    VscodeLocal,
}

impl SearchSource {
    pub fn label(&self) -> &str {
        match self {
            SearchSource::ExternalIngest(_) => "Ingest",
            SearchSource::GitHub(_) => "GitHub",
            SearchSource::VscodeLocal => "VSCode",
        }
    }

    pub fn name(&self) -> &str {
        match self {
            SearchSource::ExternalIngest(name) => name,
            SearchSource::GitHub(nwo) => nwo,
            SearchSource::VscodeLocal => "local",
        }
    }
}

impl std::fmt::Display for SearchSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.label(), self.name())
    }
}

#[derive(Debug, Clone)]
pub struct TaggedSearchResult {
    pub source: SearchSource,
    pub result: SearchResult,
}

pub struct HybridSearchResponse {
    pub results: Vec<TaggedSearchResult>,
    pub embedding_model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexStatusResponse {
    #[serde(default)]
    pub semantic_code_search_ok: bool,
    #[serde(default)]
    pub semantic_indexing_enabled: bool,
    #[serde(default)]
    pub semantic_commit_sha: Option<String>,
    #[serde(default)]
    pub can_index: Option<String>,
    #[serde(default)]
    pub lexical_search_ok: bool,
}
