//! OAuth device-flow helpers for Git hosting providers (Git sync backend).

use serde::{Deserialize, Serialize};

use crate::secret;

pub const GITHUB_ACCOUNT: &str = "oauth.github";
pub const GITLAB_ACCOUNT: &str = "oauth.gitlab";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OAuthProvider {
    Github,
    Gitlab,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceFlowStart {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub interval: u64,
    pub expires_in: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuthStatus {
    pub provider: OAuthProvider,
    pub connected: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum OAuthError {
    #[error("http: {0}")]
    Http(String),
    #[error("oauth: {0}")]
    OAuth(String),
    #[error("keyring: {0}")]
    Keyring(String),
    #[error("pending")]
    Pending,
    #[error("slow_down")]
    SlowDown,
}

impl From<reqwest::Error> for OAuthError {
    fn from(e: reqwest::Error) -> Self {
        OAuthError::Http(e.to_string())
    }
}

impl From<crate::secret::SecretError> for OAuthError {
    fn from(e: crate::secret::SecretError) -> Self {
        OAuthError::Keyring(e.to_string())
    }
}

pub fn account_for(provider: OAuthProvider) -> &'static str {
    match provider {
        OAuthProvider::Github => GITHUB_ACCOUNT,
        OAuthProvider::Gitlab => GITLAB_ACCOUNT,
    }
}

pub fn store_token(provider: OAuthProvider, token: &str) -> Result<(), OAuthError> {
    secret::set_secret(account_for(provider), token).map_err(Into::into)
}

pub fn load_token(provider: OAuthProvider) -> Result<Option<String>, OAuthError> {
    match secret::get_secret(account_for(provider)) {
        Ok(t) if !t.is_empty() => Ok(Some(t)),
        Ok(_) => Ok(None),
        Err(crate::secret::SecretError::NotFound) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn clear_token(provider: OAuthProvider) -> Result<(), OAuthError> {
    secret::clear_secret(Some(account_for(provider))).map_err(Into::into)
}

pub async fn device_start(
    provider: OAuthProvider,
    client_id: &str,
    gitlab_base_url: Option<&str>,
) -> Result<DeviceFlowStart, OAuthError> {
    match provider {
        OAuthProvider::Github => github_device_start(client_id).await,
        OAuthProvider::Gitlab => {
            gitlab_device_start(client_id, gitlab_base_url.unwrap_or("https://gitlab.com")).await
        }
    }
}

pub async fn device_poll(
    provider: OAuthProvider,
    client_id: &str,
    device_code: &str,
    gitlab_base_url: Option<&str>,
) -> Result<String, OAuthError> {
    let token = match provider {
        OAuthProvider::Github => github_device_poll(client_id, device_code).await?,
        OAuthProvider::Gitlab => {
            gitlab_device_poll(
                client_id,
                device_code,
                gitlab_base_url.unwrap_or("https://gitlab.com"),
            )
            .await?
        }
    };
    store_token(provider, &token)?;
    Ok(token)
}

#[derive(Debug, Deserialize)]
struct GithubDeviceStartResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    #[serde(default = "default_interval")]
    interval: u64,
    expires_in: u64,
}

fn default_interval() -> u64 {
    5
}

#[derive(Debug, Deserialize)]
struct GithubDevicePollResponse {
    error: Option<String>,
    access_token: Option<String>,
}

async fn github_device_start(client_id: &str) -> Result<DeviceFlowStart, OAuthError> {
    let client = reqwest::Client::new();
    let resp: GithubDeviceStartResponse = client
        .post("https://github.com/login/device/code")
        .header("Accept", "application/json")
        .form(&[
            ("client_id", client_id),
            ("scope", "repo"),
        ])
        .send()
        .await?
        .error_for_status()
        .map_err(|e| OAuthError::Http(e.to_string()))?
        .json()
        .await?;
    Ok(DeviceFlowStart {
        device_code: resp.device_code,
        user_code: resp.user_code,
        verification_uri: resp.verification_uri,
        interval: resp.interval.max(1),
        expires_in: resp.expires_in,
    })
}

async fn github_device_poll(client_id: &str, device_code: &str) -> Result<String, OAuthError> {
    let client = reqwest::Client::new();
    let resp: GithubDevicePollResponse = client
        .post("https://github.com/login/oauth/access_token")
        .header("Accept", "application/json")
        .form(&[
            ("client_id", client_id),
            ("device_code", device_code),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ])
        .send()
        .await?
        .json()
        .await?;
    match resp.error.as_deref() {
        None | Some("") => {}
        Some("authorization_pending") => return Err(OAuthError::Pending),
        Some("slow_down") => return Err(OAuthError::SlowDown),
        Some(other) => return Err(OAuthError::OAuth(other.to_string())),
    }
    resp.access_token
        .ok_or_else(|| OAuthError::OAuth("missing access_token".into()))
}

#[derive(Debug, Deserialize)]
struct GitlabDeviceStartResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    #[serde(default = "default_interval")]
    interval: u64,
    expires_in: u64,
}

#[derive(Debug, Deserialize)]
struct GitlabDevicePollResponse {
    error: Option<String>,
    access_token: Option<String>,
}

async fn gitlab_device_start(client_id: &str, base: &str) -> Result<DeviceFlowStart, OAuthError> {
    let base = base.trim_end_matches('/');
    let client = reqwest::Client::new();
    let url = format!("{base}/oauth/authorize_device");
    let resp: GitlabDeviceStartResponse = client
        .post(url)
        .form(&[
            ("client_id", client_id),
            ("scope", "write_repository"),
        ])
        .send()
        .await?
        .error_for_status()
        .map_err(|e| OAuthError::Http(e.to_string()))?
        .json()
        .await?;
    Ok(DeviceFlowStart {
        device_code: resp.device_code,
        user_code: resp.user_code,
        verification_uri: resp.verification_uri,
        interval: resp.interval.max(1),
        expires_in: resp.expires_in,
    })
}

async fn gitlab_device_poll(
    client_id: &str,
    device_code: &str,
    base: &str,
) -> Result<String, OAuthError> {
    let base = base.trim_end_matches('/');
    let client = reqwest::Client::new();
    let url = format!("{base}/oauth/token");
    let resp: GitlabDevicePollResponse = client
        .post(url)
        .form(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("client_id", client_id),
            ("device_code", device_code),
        ])
        .send()
        .await?
        .json()
        .await?;
    match resp.error.as_deref() {
        None | Some("") => {}
        Some("authorization_pending") => return Err(OAuthError::Pending),
        Some("slow_down") => return Err(OAuthError::SlowDown),
        Some(other) => return Err(OAuthError::OAuth(other.to_string())),
    }
    resp.access_token
        .ok_or_else(|| OAuthError::OAuth("missing access_token".into()))
}
