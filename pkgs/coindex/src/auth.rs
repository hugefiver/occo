use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use reqwest::header::{ACCEPT, HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

const CLIENT_ID: &str = "Iv1.b507a08c87ecfe98";
const VSCODE_CLIENT_ID: &str = "01ab8ac9400c4e429b23";

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

#[derive(Debug, Serialize)]
pub struct AuthInfo {
    pub authenticated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CoindexAuthFile {
    refresh: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    vscode_refresh: Option<String>,
}

pub async fn get_token(interactive: bool) -> Result<String> {
    if let Some(token) = read_coindex_token()? {
        return Ok(token);
    }

    if let Some(token) = read_opencode_token()? {
        return Ok(token);
    }

    if !interactive {
        bail!("not authenticated; run `coindex auth` to set up");
    }

    let token = device_flow_token().await?;
    save_coindex_token(&token)?;
    Ok(token)
}

pub async fn run_auth(interactive: bool, force: bool) -> Result<AuthInfo> {
    if !force {
        if read_coindex_token()?.is_some() {
            return Ok(AuthInfo {
                authenticated: true,
                source: Some("coindex".to_string()),
            });
        }

        if read_opencode_token()?.is_some() {
            return Ok(AuthInfo {
                authenticated: true,
                source: Some("opencode".to_string()),
            });
        }
    }

    if !interactive {
        if force {
            bail!("--force requires interactive mode (cannot run in JSON/Markdown output mode)");
        }
        return Ok(AuthInfo {
            authenticated: false,
            source: None,
        });
    }

    let token = device_flow_token().await?;
    save_coindex_token(&token)?;
    Ok(AuthInfo {
        authenticated: true,
        source: Some("coindex (new)".to_string()),
    })
}

fn auth_file_candidates() -> Result<Vec<PathBuf>> {
    let home = dirs::home_dir().context("failed to resolve home directory")?;
    let mut candidates = Vec::new();

    // XDG-style (OpenCode actually uses this on all platforms)
    candidates.push(
        home.join(".local")
            .join("share")
            .join("opencode")
            .join("auth.json"),
    );

    if cfg!(target_os = "windows") {
        if let Ok(app_data) = std::env::var("APPDATA") {
            candidates.push(PathBuf::from(app_data).join("opencode").join("auth.json"));
        }
    } else if cfg!(target_os = "macos") {
        candidates.push(
            home.join("Library")
                .join("Application Support")
                .join("opencode")
                .join("auth.json"),
        );
    }

    Ok(candidates)
}

fn coindex_auth_file() -> Result<PathBuf> {
    let home = dirs::home_dir().context("failed to resolve home directory")?;
    Ok(home
        .join(".local")
        .join("share")
        .join("coindex")
        .join("auth.json"))
}

fn read_coindex_token() -> Result<Option<String>> {
    let path = coindex_auth_file()?;
    if !path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let parsed: CoindexAuthFile = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    let token = parsed.refresh.trim().to_string();
    if token.is_empty() {
        return Ok(None);
    }
    info!("found token at {}", path.display());
    Ok(Some(token))
}

pub fn save_coindex_token(token: &str) -> Result<()> {
    let path = coindex_auth_file()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let existing_vscode = if path.exists() {
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str::<CoindexAuthFile>(&raw).ok())
            .and_then(|f| f.vscode_refresh)
    } else {
        None
    };
    let data = CoindexAuthFile {
        refresh: token.to_string(),
        vscode_refresh: existing_vscode,
    };
    let json = serde_json::to_string_pretty(&data)?;
    std::fs::write(&path, &json).with_context(|| format!("failed to write {}", path.display()))?;
    info!("saved token to {}", path.display());
    Ok(())
}

pub fn read_vscode_token() -> Result<Option<String>> {
    let path = coindex_auth_file()?;
    if !path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let parsed: CoindexAuthFile = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    match parsed.vscode_refresh {
        Some(t) if !t.trim().is_empty() => {
            info!("found VS Code token at {}", path.display());
            Ok(Some(t.trim().to_string()))
        }
        _ => Ok(None),
    }
}

pub fn save_vscode_token(token: &str) -> Result<()> {
    let path = coindex_auth_file()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let existing_refresh = if path.exists() {
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str::<CoindexAuthFile>(&raw).ok())
            .map(|f| f.refresh)
            .unwrap_or_default()
    } else {
        String::new()
    };
    if existing_refresh.is_empty() {
        bail!("cannot save VS Code token: no primary token found in {}", path.display());
    }
    let data = CoindexAuthFile {
        refresh: existing_refresh,
        vscode_refresh: Some(token.to_string()),
    };
    let json = serde_json::to_string_pretty(&data)?;
    std::fs::write(&path, &json).with_context(|| format!("failed to write {}", path.display()))?;
    info!("saved VS Code token to {}", path.display());
    Ok(())
}

fn read_opencode_token() -> Result<Option<String>> {
    let candidates = auth_file_candidates()?;
    for path in &candidates {
        if !path.exists() {
            continue;
        }

        let raw = match std::fs::read_to_string(path) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let parsed: AuthFile = match serde_json::from_str(&raw) {
            Ok(p) => p,
            Err(_) => continue,
        };

        if let Some(token) = parsed
            .occo
            .and_then(|o| o.refresh)
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
        {
            info!("found token at {}", path.display());
            return Ok(Some(token));
        }
    }

    Ok(None)
}

async fn device_flow_token() -> Result<String> {
    device_flow_token_with_scope("user:email").await
}

pub async fn device_flow_token_with_scope(scope: &str) -> Result<String> {
    device_flow_token_with_client(CLIENT_ID, scope).await
}

pub async fn vscode_device_flow_token(scope: &str) -> Result<String> {
    device_flow_token_with_client(VSCODE_CLIENT_ID, scope).await
}

async fn device_flow_token_with_client(client_id: &str, scope: &str) -> Result<String> {
    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));

    let http = reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .context("failed to build HTTP client for device flow")?;

    let device: DeviceCodeResponse = http
        .post("https://github.com/login/device/code")
        .form(&[("client_id", client_id), ("scope", scope)])
        .send()
        .await
        .context("failed to request device code")?
        .error_for_status()
        .context("device code endpoint returned error")?
        .json()
        .await
        .context("failed to decode device code response")?;

    info!(
        url = %device.verification_uri,
        code = %device.user_code,
        "open URL and enter the code"
    );

    let mut interval = device.interval.unwrap_or(5).max(1);
    let grant_type = "urn:ietf:params:oauth:grant-type:device_code";

    loop {
        tokio::time::sleep(Duration::from_secs(interval)).await;

        let resp: AccessTokenResponse = http
            .post("https://github.com/login/oauth/access_token")
            .form(&[
                ("client_id", client_id),
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
