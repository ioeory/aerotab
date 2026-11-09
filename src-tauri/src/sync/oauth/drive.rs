//! Public-client Google OAuth PKCE. No client secret is accepted or persisted.
use crate::{
    secret,
    sync::{backends::cloud, SyncError},
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub const ACCOUNT: &str = "sync.drive.oauth";
const SCOPE: &str = "https://www.googleapis.com/auth/drive.appdata";

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn callback_ignores_wrong_state_and_exchanges_pkce_only_after_valid_loopback() {
        let flow = Pending::start("mock-client".into()).await.unwrap();
        let address = flow.listener.local_addr().unwrap();
        let state = flow.state.clone();
        let verifier = flow.verifier.clone();
        let auth = reqwest::Url::parse(&flow.url).unwrap();
        let query: std::collections::HashMap<_, _> = auth.query_pairs().into_owned().collect();
        assert_eq!(
            query["code_challenge"],
            URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
        );
        assert_eq!(query["scope"], SCOPE);
        let (endpoint, server) = cloud::test_http::server(vec![(
            200,
            r#"{"access_token":"mock-access","refresh_token":"mock-refresh","expires_in":3600}"#
                .into(),
        )])
        .await;
        let exchange = tokio::spawn(async move {
            flow.exchange(&endpoint, std::time::Duration::from_secs(3))
                .await
        });
        let client = reqwest::Client::new();
        assert_eq!(
            client
                .get(format!("http://{address}/oauth/callback?state={state}"))
                .send()
                .await
                .unwrap()
                .status(),
            400
        );
        let mut malformed = tokio::net::TcpStream::connect(address).await.unwrap();
        malformed
            .write_all(b"GET http://[invalid HTTP/1.1\r\n\r\n")
            .await
            .unwrap();
        drop(malformed);
        assert_eq!(
            client
                .get(format!(
                    "http://{address}/oauth/callback?state=wrong&code=evil"
                ))
                .send()
                .await
                .unwrap()
                .status(),
            400
        );
        assert_eq!(
            client
                .get(format!(
                    "http://{address}/oauth/callback?state={state}&code=good"
                ))
                .send()
                .await
                .unwrap()
                .status(),
            200
        );
        let token = exchange.await.unwrap().unwrap();
        assert_eq!(token.refresh_token, "mock-refresh");
        let requests = server.await.unwrap();
        assert!(requests[0].contains(&format!("code_verifier={verifier}")));
        assert!(requests[0].contains("code=good"));
        assert!(!requests[0].contains("client_secret"));
    }
    #[tokio::test]
    async fn callback_timeout_does_not_exchange_or_store_tokens() {
        let flow = Pending::start("mock-client".into()).await.unwrap();
        assert!(flow
            .exchange("http://127.0.0.1:1", std::time::Duration::from_millis(10))
            .await
            .is_err());
    }
    #[tokio::test]
    async fn refresh_uses_refresh_grant_and_never_requires_desktop_secret() {
        let (endpoint, server) = cloud::test_http::server(vec![(
            200,
            r#"{"access_token":"new-access","expires_in":3600}"#.into(),
        )])
        .await;
        let token = Token {
            access_token: "expired".into(),
            refresh_token: "mock-refresh".into(),
            expires_at: 0,
            client_id: "mock-client".into(),
        };
        let token = refresh_token(token, "mock-client", &endpoint)
            .await
            .unwrap();
        assert_eq!(token.access_token, "new-access");
        assert_eq!(token.refresh_token, "mock-refresh");
        let requests = server.await.unwrap();
        assert!(requests[0].contains("grant_type=refresh_token"));
        assert!(!requests[0].contains("client_secret"));
    }
}
#[derive(Serialize, Deserialize)]
pub struct Token {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
    pub client_id: String,
}
fn random() -> String {
    let mut b = [0; 32];
    rand::thread_rng().fill_bytes(&mut b);
    URL_SAFE_NO_PAD.encode(b)
}
fn save(token: &Token) -> Result<(), SyncError> {
    secret::set_secret(
        ACCOUNT,
        &serde_json::to_string(token).map_err(cloud::transport)?,
    )
    .map_err(|_| SyncError::Transport("Drive token keyring write failed".into()))
}
pub async fn refresh(client_id: &str) -> Result<Token, SyncError> {
    let raw = secret::get_secret(ACCOUNT).map_err(|_| SyncError::Auth)?;
    let token: Token = serde_json::from_str(&raw).map_err(cloud::transport)?;
    let token = refresh_token(token, client_id, "https://oauth2.googleapis.com/token").await?;
    save(&token)?;
    Ok(token)
}
pub async fn refresh_token(
    mut token: Token,
    client_id: &str,
    endpoint: &str,
) -> Result<Token, SyncError> {
    if token.client_id != client_id {
        return Err(SyncError::Auth);
    }
    if token.expires_at > chrono::Utc::now().timestamp() + 60 {
        return Ok(token);
    }
    let client = cloud::client()?;
    let response = cloud::send(
        &client,
        client
            .post(endpoint)
            .form(&[
                ("client_id", client_id),
                ("refresh_token", &token.refresh_token),
                ("grant_type", "refresh_token"),
            ])
            .build()
            .map_err(cloud::transport)?,
    )
    .await?;
    let value: serde_json::Value = response.json().await.map_err(cloud::transport)?;
    token.access_token = value["access_token"]
        .as_str()
        .ok_or(SyncError::Auth)?
        .into();
    if let Some(refresh) = value["refresh_token"].as_str().filter(|s| !s.is_empty()) {
        token.refresh_token = refresh.into();
    }
    token.expires_at =
        chrono::Utc::now().timestamp() + value["expires_in"].as_i64().unwrap_or(3600);
    Ok(token)
}

pub struct Pending {
    pub url: String,
    pub listener: tokio::net::TcpListener,
    pub verifier: String,
    pub state: String,
    pub redirect: String,
    pub client_id: String,
}
impl Pending {
    pub async fn start(client_id: String) -> Result<Self, SyncError> {
        if client_id.trim().is_empty() {
            return Err(SyncError::Transport(
                "Google Desktop OAuth client ID required".into(),
            ));
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(cloud::transport)?;
        let redirect = format!(
            "http://127.0.0.1:{}/oauth/callback",
            listener.local_addr().map_err(cloud::transport)?.port()
        );
        let verifier = random();
        let state = random();
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let mut url = reqwest::Url::parse("https://accounts.google.com/o/oauth2/v2/auth")
            .map_err(cloud::transport)?;
        url.query_pairs_mut().extend_pairs(&[
            ("client_id", client_id.as_str()),
            ("redirect_uri", redirect.as_str()),
            ("response_type", "code"),
            ("scope", SCOPE),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("state", state.as_str()),
            ("access_type", "offline"),
            ("prompt", "consent"),
        ]);
        Ok(Self {
            url: url.into(),
            listener,
            verifier,
            state,
            redirect,
            client_id,
        })
    }
    pub async fn finish(self) -> Result<(), SyncError> {
        let token = self
            .exchange(
                "https://oauth2.googleapis.com/token",
                std::time::Duration::from_secs(180),
            )
            .await?;
        secret::set_secret(
            "sync.drive.authGeneration",
            &uuid::Uuid::new_v4().to_string(),
        )
        .map_err(|_| SyncError::Transport("Drive auth generation keyring write failed".into()))?;
        save(&token)
    }
    async fn exchange(
        self,
        endpoint: &str,
        timeout: std::time::Duration,
    ) -> Result<Token, SyncError> {
        let deadline = tokio::time::Instant::now() + timeout;
        let code = loop {
            let (mut stream, peer) = tokio::time::timeout_at(deadline, self.listener.accept())
                .await
                .map_err(cloud::transport)?
                .map_err(cloud::transport)?;
            if !peer.ip().is_loopback() {
                continue;
            }
            let mut bytes = vec![0; 8192];
            let read_deadline =
                deadline.min(tokio::time::Instant::now() + std::time::Duration::from_secs(5));
            let n = match tokio::time::timeout_at(read_deadline, stream.read(&mut bytes)).await {
                Ok(Ok(n)) if n > 0 => n,
                _ => continue,
            };
            let request = String::from_utf8_lossy(&bytes[..n]);
            let target = request
                .lines()
                .next()
                .and_then(|s| s.strip_prefix("GET "))
                .and_then(|s| s.split_whitespace().next())
                .unwrap_or("");
            let Ok(parsed) = reqwest::Url::parse(&format!("http://localhost{target}")) else {
                continue;
            };
            let pairs: Vec<_> = parsed.query_pairs().into_owned().collect();
            let params: std::collections::HashMap<_, _> = pairs.iter().cloned().collect();
            let valid = pairs.len() == params.len()
                && target.starts_with('/')
                && !target.starts_with("//")
                && parsed.path() == "/oauth/callback"
                && params.get("state") == Some(&self.state)
                && params.get("code").is_some_and(|code| !code.is_empty());
            let response = if valid {
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\nAuthorization received. Return to AeroTab."
            } else {
                "HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\nInvalid authorization callback."
            };
            let _ = tokio::time::timeout_at(deadline, stream.write_all(response.as_bytes())).await;
            if !valid {
                continue;
            }
            break params.get("code").cloned().ok_or(SyncError::Auth)?;
        };
        let client = cloud::client()?;
        let request = client
            .post(endpoint)
            .form(&[
                ("client_id", self.client_id.as_str()),
                ("code", code.as_str()),
                ("code_verifier", self.verifier.as_str()),
                ("redirect_uri", self.redirect.as_str()),
                ("grant_type", "authorization_code"),
            ])
            .build()
            .map_err(cloud::transport)?;
        let value: serde_json::Value = cloud::send(&client, request)
            .await?
            .json()
            .await
            .map_err(cloud::transport)?;
        let token = Token {
            access_token: value["access_token"]
                .as_str()
                .ok_or(SyncError::Auth)?
                .into(),
            refresh_token: value["refresh_token"]
                .as_str()
                .ok_or(SyncError::Auth)?
                .into(),
            expires_at: chrono::Utc::now().timestamp()
                + value["expires_in"].as_i64().unwrap_or(3600),
            client_id: self.client_id,
        };
        Ok(token)
    }
}
