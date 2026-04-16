use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use futures::stream::{FuturesUnordered, StreamExt};
use reqwest::StatusCode;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue, RETRY_AFTER};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tracing::debug;

use crate::types::{
    BatchRequest, BatchResponse, CreateIngestRequest, CreateIngestResponse, DeleteFilesetRequest,
    FinalizeRequest, ListFilesetsResponse, SearchRequest, SearchResponse, UploadDocumentRequest,
    UploadSymbolsRequest, UploadSymbolsResponse,
};

const BASE_URL: &str = "https://api.github.com";
const MAX_UPLOAD_CONCURRENCY: usize = 64;

#[derive(Clone)]
pub struct IngestClient {
    pub http: reqwest::Client,
    #[allow(dead_code)]
    pub token: String,
}

impl IngestClient {
    pub fn new(token: String) -> Result<Self> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let bearer = format!("Bearer {token}");
        let mut auth =
            HeaderValue::from_str(&bearer).context("failed to create authorization header")?;
        auth.set_sensitive(true);
        headers.insert(AUTHORIZATION, auth);
        headers.insert(
            "X-Client-Application",
            HeaderValue::from_static("vscode/1.115.0"),
        );
        headers.insert(
            "X-Client-Source",
            HeaderValue::from_static("copilot-chat/0.43.0"),
        );

        let http = reqwest::Client::builder()
            .user_agent("vscode/1.115.0")
            .default_headers(headers)
            .build()
            .context("failed to build HTTP client")?;

        Ok(Self { http, token })
    }

    pub async fn create_ingest(&self, req: CreateIngestRequest) -> Result<CreateIngestResponse> {
        let url = self.url("/external/code/ingest");
        let mut conflict_retries = 0u32;
        let mut attempts = 0u32;

        debug!(
            fileset = %req.fileset_name,
            checkpoint = %req.new_checkpoint,
            geo_filter_len = req.geo_filter.len(),
            coded_symbols_count = req.coded_symbols.len(),
            "create_ingest request"
        );
        if let Ok(body) = serde_json::to_string(&req) {
            debug!(body = %body, "create_ingest body");
        }

        loop {
            attempts += 1;
            if attempts > 20 {
                bail!("create_ingest exceeded retry attempts");
            }

            let response = self
                .http
                .post(&url)
                .json(&req)
                .send()
                .await
                .context("create_ingest request failed")?;
            let status = response.status();

            if status.is_success() {
                let payload: CreateIngestResponse = response
                    .json()
                    .await
                    .context("failed to decode create_ingest response")?;
                if payload.ingest_id.is_empty()
                    && payload.coded_symbol_range.start == 0
                    && payload.coded_symbol_range.end == 0
                {
                    return Ok(payload);
                }
                return Ok(payload);
            }

            if status == StatusCode::CONFLICT {
                conflict_retries += 1;
                if conflict_retries > 3 {
                    let body = response_text(response).await;
                    bail!("create_ingest conflict after retries: {body}");
                }
                let delay = 1u64 << conflict_retries;
                tokio::time::sleep(Duration::from_secs(delay)).await;
                continue;
            }

            if status == StatusCode::TOO_MANY_REQUESTS {
                if let Some(wait_secs) = retry_after_secs(response.headers()) {
                    tokio::time::sleep(Duration::from_secs(wait_secs)).await;
                    continue;
                }

                let filesets = self
                    .list_filesets()
                    .await
                    .context("failed to list filesets after 429")?;
                if let Some(oldest) = filesets.filesets.first() {
                    self.delete_fileset(DeleteFilesetRequest {
                        fileset_name: oldest.name.clone(),
                    })
                    .await
                    .context("failed to delete oldest fileset after 429")?;
                    continue;
                }

                bail!("create_ingest got 429 without Retry-After and no filesets available");
            }

            let body = response_text(response).await;
            bail!("create_ingest failed with status {status}: {body}");
        }
    }

    #[allow(dead_code)]
    pub async fn upload_symbols(&self, req: UploadSymbolsRequest) -> Result<UploadSymbolsResponse> {
        self.post_json_with_rate_limit(
            "/external/code/ingest/coded_symbols",
            &req,
            "upload_symbols",
        )
        .await
    }

    #[allow(dead_code)]
    pub async fn get_batch(&self, req: BatchRequest) -> Result<BatchResponse> {
        self.post_json_with_rate_limit("/external/code/ingest/batch", &req, "get_batch")
            .await
    }

    pub async fn upload_document(&self, req: UploadDocumentRequest) -> Result<()> {
        let url = self.url("/external/code/ingest/document");
        let mut throttled_retries = 0u32;
        let mut server_retries = 0u32;

        loop {
            let response = match self.http.post(&url).json(&req).send().await {
                Ok(resp) => resp,
                Err(err) => {
                    if server_retries >= 3 {
                        return Err(anyhow!(
                            "upload_document network error after {} retries: {err}",
                            server_retries
                        ));
                    }
                    server_retries += 1;
                    let backoff = 1u64 << server_retries;
                    tokio::time::sleep(Duration::from_secs(backoff)).await;
                    continue;
                }
            };

            let status = response.status();

            if status.is_success() {
                return Ok(());
            }

            if status == StatusCode::CONFLICT || status == StatusCode::NOT_FOUND {
                let body = response_text(response).await;
                return Err(anyhow!("upload_document hard failure {status}: {body}"));
            }

            if status == StatusCode::TOO_MANY_REQUESTS {
                if throttled_retries >= 10 {
                    return Err(anyhow!(
                        "upload_document failed: 429 retries exhausted for doc {}",
                        req.doc_id
                    ));
                }
                throttled_retries += 1;
                let wait = retry_after_secs(response.headers()).unwrap_or(1);
                tokio::time::sleep(Duration::from_secs(wait)).await;
                continue;
            }

            if status.is_server_error() {
                if server_retries >= 3 {
                    let body = response_text(response).await;
                    return Err(anyhow!(
                        "upload_document failed after 5xx retries: {status} {body}"
                    ));
                }
                server_retries += 1;
                let wait = retry_after_secs(response.headers()).unwrap_or(1u64 << server_retries);
                tokio::time::sleep(Duration::from_secs(wait)).await;
                continue;
            }

            let body = response_text(response).await;
            return Err(anyhow!(
                "upload_document unexpected status {status}: {body}"
            ));
        }
    }

    pub async fn upload_documents_concurrent(
        &self,
        requests: Vec<UploadDocumentRequest>,
    ) -> Result<()> {
        let mut iter = requests.into_iter();
        let mut in_flight = FuturesUnordered::new();

        for _ in 0..MAX_UPLOAD_CONCURRENCY {
            if let Some(req) = iter.next() {
                in_flight.push(self.upload_document(req));
            }
        }

        while let Some(result) = in_flight.next().await {
            result?;
            if let Some(req) = iter.next() {
                in_flight.push(self.upload_document(req));
            }
        }

        Ok(())
    }

    pub async fn finalize(&self, req: FinalizeRequest) -> Result<()> {
        let url = self.url("/external/code/ingest/finalize");
        self.post_empty_with_rate_limit(&url, &req, "finalize")
            .await
    }

    pub async fn list_filesets(&self) -> Result<ListFilesetsResponse> {
        let url = self.url("/external/code/ingest");
        let mut attempts = 0u32;
        loop {
            attempts += 1;
            if attempts > 10 {
                bail!("list_filesets exceeded retry attempts");
            }

            let response = self
                .http
                .get(&url)
                .send()
                .await
                .context("list_filesets request failed")?;

            let status = response.status();
            if status.is_success() {
                let payload: ListFilesetsResponse = response
                    .json()
                    .await
                    .context("failed to decode list_filesets response")?;
                return Ok(payload);
            }

            if status == StatusCode::TOO_MANY_REQUESTS
                && let Some(wait) = retry_after_secs(response.headers())
            {
                tokio::time::sleep(Duration::from_secs(wait)).await;
                continue;
            }

            let body = response_text(response).await;
            bail!("list_filesets failed with status {status}: {body}");
        }
    }

    pub async fn delete_fileset(&self, req: DeleteFilesetRequest) -> Result<()> {
        let url = self.url("/external/code/ingest");
        let mut attempts = 0u32;
        loop {
            attempts += 1;
            if attempts > 10 {
                bail!("delete_fileset exceeded retry attempts");
            }

            let response = self
                .http
                .delete(&url)
                .json(&req)
                .send()
                .await
                .context("delete_fileset request failed")?;

            let status = response.status();
            if status.is_success() {
                return Ok(());
            }

            if status == StatusCode::TOO_MANY_REQUESTS
                && let Some(wait) = retry_after_secs(response.headers())
            {
                tokio::time::sleep(Duration::from_secs(wait)).await;
                continue;
            }

            let body = response_text(response).await;
            bail!("delete_fileset failed with status {status}: {body}");
        }
    }

    pub async fn search(&self, req: SearchRequest) -> Result<SearchResponse> {
        self.post_json_with_rate_limit("/external/embeddings/code/search", &req, "search")
            .await
    }

    fn url(&self, path: &str) -> String {
        format!("{BASE_URL}{path}")
    }

    async fn post_json_with_rate_limit<TReq, TRes>(
        &self,
        path: &str,
        payload: &TReq,
        op_name: &str,
    ) -> Result<TRes>
    where
        TReq: Serialize + ?Sized,
        TRes: DeserializeOwned,
    {
        let url = self.url(path);
        let mut attempts = 0u32;

        loop {
            attempts += 1;
            if attempts > 10 {
                bail!("{op_name} exceeded retry attempts");
            }

            let response = self
                .http
                .post(&url)
                .json(payload)
                .send()
                .await
                .with_context(|| format!("{op_name} request failed"))?;

            let status = response.status();
            if status.is_success() {
                let value = response
                    .json::<TRes>()
                    .await
                    .with_context(|| format!("failed to decode {op_name} response"))?;
                return Ok(value);
            }

            if status == StatusCode::TOO_MANY_REQUESTS
                && let Some(wait) = retry_after_secs(response.headers())
            {
                tokio::time::sleep(Duration::from_secs(wait)).await;
                continue;
            }

            let body = response_text(response).await;
            bail!("{op_name} failed with status {status}: {body}");
        }
    }

    async fn post_empty_with_rate_limit<TReq>(
        &self,
        url: &str,
        payload: &TReq,
        op_name: &str,
    ) -> Result<()>
    where
        TReq: Serialize + ?Sized,
    {
        let mut attempts = 0u32;
        loop {
            attempts += 1;
            if attempts > 10 {
                bail!("{op_name} exceeded retry attempts");
            }

            let response = self
                .http
                .post(url)
                .json(payload)
                .send()
                .await
                .with_context(|| format!("{op_name} request failed"))?;

            let status = response.status();
            if status.is_success() {
                return Ok(());
            }

            if status == StatusCode::TOO_MANY_REQUESTS
                && let Some(wait) = retry_after_secs(response.headers())
            {
                tokio::time::sleep(Duration::from_secs(wait)).await;
                continue;
            }

            let body = response_text(response).await;
            bail!("{op_name} failed with status {status}: {body}");
        }
    }
}

fn retry_after_secs(headers: &HeaderMap) -> Option<u64> {
    headers
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|raw| raw.trim().parse::<u64>().ok())
}

async fn response_text(response: reqwest::Response) -> String {
    response.text().await.unwrap_or_default()
}
