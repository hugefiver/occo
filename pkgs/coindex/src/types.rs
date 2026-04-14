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
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Location {
    pub fileset: String,
    pub checkpoint: String,
    pub doc_id: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Chunk {
    pub hash: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_range: Option<(u32, u32)>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<(u32, u32)>,
}
