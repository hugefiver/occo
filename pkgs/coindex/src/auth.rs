use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use reqwest::header::{ACCEPT, HeaderMap, HeaderValue};
use serde::Deserialize;
use tracing::{info, warn};

const CLIENT_ID: &str = "Iv1.b507a08c87ecfe98";

#[derive(Debug, Deserialize)]
struct AuthFile {
    occo: Option<OccoAuth>,
}

#[derive(Debug, Deserialize)]
struct OccoAuth {
    refresh: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    interval: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct AccessTokenResponse {
    access_token: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
    interval: Option<u64>,
}

pub async fn get_token() -> Result<String> {
    if let Some(token) = read_opencode_token()? {
        return Ok(token);
    }

    device_flow_token().await
}

pub async fn auth_status() -> Result<()> {
    if read_opencode_token()?.is_some() {
        println!("auth: opencode token (auth.json)");
        return Ok(());
    }

    println!("auth: opencode token not found, device flow required");
    Ok(())
}

fn auth_file_path() -> Result<PathBuf> {
    if cfg!(target_os = "windows") {
        let app_data = std::env::var("APPDATA").context("APPDATA is not set")?;
        return Ok(PathBuf::from(app_data).join("opencode").join("auth.json"));
    }

    let home = dirs::home_dir().context("failed to resolve home directory")?;

    if cfg!(target_os = "macos") {
        return Ok(home
            .join("Library")
            .join("Application Support")
            .join("opencode")
            .join("auth.json"));
    }

    if cfg!(unix) {
        return Ok(home
            .join(".local")
            .join("share")
            .join("opencode")
            .join("auth.json"));
    }

    Err(anyhow!("unsupported platform"))
}

fn read_opencode_token() -> Result<Option<String>> {
    let path = auth_file_path()?;
    if !path.exists() {
        return Ok(None);
    }

    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read auth file at {}", path.display()))?;
    let parsed: AuthFile = serde_json::from_str(&raw).with_context(|| {
        format!(
            "failed to parse opencode auth file JSON at {}",
            path.display()
        )
    })?;

    Ok(parsed
        .occo
        .and_then(|o| o.refresh)
        .map(|token| token.trim().to_string())
        .filter(|token| !token.is_empty()))
}

async fn device_flow_token() -> Result<String> {
    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));

    let http = reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .context("failed to build HTTP client for device flow")?;

    let device: DeviceCodeResponse = http
        .post("https://github.com/login/device/code")
        .form(&[("client_id", CLIENT_ID), ("scope", "user:email")])
        .send()
        .await
        .context("failed to request device code")?
        .error_for_status()
        .context("device code endpoint returned error")?
        .json()
        .await
        .context("failed to decode device code response")?;

    println!(
        "Open {} and enter code {}",
        device.verification_uri, device.user_code
    );

    let mut interval = device.interval.unwrap_or(5).max(1);
    let grant_type = "urn:ietf:params:oauth:grant-type:device_code";

    loop {
        tokio::time::sleep(Duration::from_secs(interval)).await;

        let resp: AccessTokenResponse = http
            .post("https://github.com/login/oauth/access_token")
            .form(&[
                ("client_id", CLIENT_ID),
                ("device_code", device.device_code.as_str()),
                ("grant_type", grant_type),
            ])
            .send()
            .await
            .context("failed to poll access token")?
            .error_for_status()
            .context("access token endpoint returned error")?
            .json()
            .await
            .context("failed to decode access token response")?;

        if let Some(token) = resp.access_token {
            info!("received token via device flow");
            return Ok(token);
        }

        if let Some(next) = resp.interval {
            interval = next.max(1);
        }

        match resp.error.as_deref() {
            Some("authorization_pending") => continue,
            Some("slow_down") => {
                interval += 5;
                continue;
            }
            Some("expired_token") => bail!("device flow expired; run auth again"),
            Some("access_denied") => bail!("device flow denied by user"),
            Some(other) => {
                let msg = resp
                    .error_description
                    .unwrap_or_else(|| "unknown device flow error".to_string());
                bail!("device flow failed: {other}: {msg}");
            }
            None => {
                warn!("device flow poll returned no token and no error");
                return Err(anyhow!("invalid device flow response"));
            }
        }
    }
}
