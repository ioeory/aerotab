//! Shared cloud transport: bounded retries, sanitized failures, and legacy record adapter.
use super::super::{blob::BlobStore, Group, RecordId, SyncBackend, SyncError};
use async_trait::async_trait;
use reqwest::{Client, Request, Response, StatusCode};
use std::{sync::Arc, time::Duration};

pub fn client() -> Result<Client, SyncError> {
    Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| SyncError::Transport("cloud HTTP client unavailable".into()))
}

pub async fn send(client: &Client, request: Request) -> Result<Response, SyncError> {
    for attempt in 0..3 {
        let retry = request
            .try_clone()
            .ok_or_else(|| SyncError::Transport("non-repeatable cloud request".into()))?;
        match client.execute(retry).await {
            Ok(response) => {
                let status = response.status();
                if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
                    return Err(SyncError::Auth);
                }
                if (status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error())
                    && attempt < 2
                {
                    let delay = response
                        .headers()
                        .get("retry-after")
                        .and_then(|h| h.to_str().ok())
                        .and_then(|s| s.parse::<u64>().ok())
                        .unwrap_or(1 << attempt)
                        .min(30);
                    tokio::time::sleep(Duration::from_secs(delay)).await;
                    continue;
                }
                if status.is_success()
                    || (status == StatusCode::NOT_FOUND
                        && matches!(
                            *request.method(),
                            reqwest::Method::GET | reqwest::Method::DELETE
                        ))
                {
                    return Ok(response);
                }
                return Err(SyncError::Transport(format!(
                    "cloud HTTP {}",
                    status.as_u16()
                )));
            }
            Err(_) if attempt < 2 => {
                tokio::time::sleep(Duration::from_millis(200 << attempt)).await
            }
            Err(_) => {
                return Err(SyncError::Transport(
                    "cloud request failed (network/timeout)".into(),
                ))
            }
        }
    }
    unreachable!()
}

pub fn transport(_: impl std::fmt::Display) -> SyncError {
    SyncError::Transport("invalid cloud response".into())
}

/// Adapts the same active object namespace to legacy record operations.
pub struct CloudRecords(pub Arc<dyn BlobStore>);

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
pub(crate) mod test_http {
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    pub async fn server(
        responses: Vec<(u16, String)>,
    ) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let mut requests = Vec::new();
            for (status, body) in responses {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                loop {
                    let mut buf = [0; 4096];
                    let n = socket.read(&mut buf).await.unwrap();
                    if n == 0 {
                        break;
                    }
                    request.extend_from_slice(&buf[..n]);
                    if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&request[..end]);
                        let size = headers
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .and_then(|n| n.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if request.len() >= end + 4 + size {
                            break;
                        }
                    }
                }
                requests.push(String::from_utf8_lossy(&request).into_owned());
                let response = format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
                socket.write_all(response.as_bytes()).await.unwrap();
            }
            requests
        });
        (address, task)
    }
}
#[async_trait]
impl SyncBackend for CloudRecords {
    async fn list(&self, group: Group) -> Result<Vec<RecordId>, SyncError> {
        let prefix = format!("{}/", group.as_str());
        Ok(self
            .0
            .list_blobs(&prefix)
            .await?
            .iter()
            .filter_map(|m| {
                m.path
                    .strip_prefix(&prefix)?
                    .strip_suffix(".bin")?
                    .parse()
                    .ok()
                    .map(RecordId)
            })
            .collect())
    }
    async fn get(&self, group: Group, id: RecordId) -> Result<Vec<u8>, SyncError> {
        self.0
            .get_blob(&format!("{}/{}.bin", group.as_str(), id.0))
            .await
    }
    async fn put(&self, group: Group, id: RecordId, bytes: &[u8]) -> Result<(), SyncError> {
        self.0
            .put_blob(&format!("{}/{}.bin", group.as_str(), id.0), bytes)
            .await
    }
    async fn delete(&self, group: Group, id: RecordId) -> Result<(), SyncError> {
        self.0
            .delete_blob(&format!("{}/{}.bin", group.as_str(), id.0))
            .await
    }
}
