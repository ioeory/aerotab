//! Google Drive appDataFolder namespace. Paths are opaque appProperties, not folders.
use super::cloud;
use crate::sync::{
    blob::{normalize_blob_path, BlobMeta, BlobStore},
    oauth::drive,
    SyncError,
};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use tokio::sync::Mutex;
pub struct DriveBackend {
    client_id: String,
    api_base: String,
    upload_base: String,
    prefix: String,
    client: reqwest::Client,
    token: Mutex<drive::Token>,
    writes: Mutex<()>,
}
#[derive(Deserialize)]
struct File {
    id: String,
    #[serde(default)]
    size: Option<String>,
    #[serde(default, rename = "appProperties")]
    properties: std::collections::HashMap<String, String>,
}
#[derive(Deserialize)]
struct Page {
    #[serde(default)]
    files: Vec<File>,
    #[serde(rename = "nextPageToken")]
    next: Option<String>,
}
#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn appdata_listing_paginates_and_deduplicates_ids_without_cross_prefix_leaks() {
        let (url,task)=cloud::test_http::server(vec![
            (200,r#"{"files":[{"id":"a","size":"3","appProperties":{"aerotabPath":"app/v2/a"}}],"nextPageToken":"next"}"#.into()),
            (200,r#"{"files":[{"id":"a","size":"3","appProperties":{"aerotabPath":"app/v2/a"}},{"id":"b","size":"7","appProperties":{"aerotabPath":"other/secret"}}]}"#.into())
        ]).await;
        let store = DriveBackend {
            client_id: "mock".into(),
            api_base: url.clone(),
            upload_base: url,
            prefix: "app".into(),
            client: cloud::client().unwrap(),
            token: Mutex::new(drive::Token {
                access_token: "mock-token".into(),
                refresh_token: "mock-refresh".into(),
                expires_at: i64::MAX,
                client_id: "mock".into(),
            }),
            writes: Mutex::new(()),
        };
        let files = store.list_blobs("v2/").await.unwrap();
        assert_eq!(
            files,
            vec![BlobMeta {
                path: "v2/a".into(),
                size: 3
            }]
        );
        let requests = task.await.unwrap();
        assert!(requests[0].contains("spaces=appDataFolder"));
        assert!(requests[1].contains("pageToken=next"));
        assert!(requests[0].contains("Bearer mock-token"));
    }
    #[tokio::test]
    async fn lost_create_ack_conflict_is_recovered_by_generated_id() {
        let (url, task) = cloud::test_http::server(vec![
            (200, r#"{"files":[]}"#.into()),
            (200, r#"{"ids":["generated"]}"#.into()),
            (409, "already created".into()),
            (200, "same".into()),
            (200, r#"{"files":[{"id":"generated"}]}"#.into()),
            (200, "same".into()),
        ])
        .await;
        let store = DriveBackend {
            client_id: "mock".into(),
            api_base: url.clone(),
            upload_base: url,
            prefix: "app".into(),
            client: cloud::client().unwrap(),
            token: Mutex::new(drive::Token {
                access_token: "mock".into(),
                refresh_token: "mock".into(),
                expires_at: i64::MAX,
                client_id: "mock".into(),
            }),
            writes: Mutex::new(()),
        };
        store
            .put_blob("v2/devices/a/ops/1.bin", b"same")
            .await
            .unwrap();
        let requests = task.await.unwrap();
        assert!(requests[3].starts_with("GET /files/generated?alt=media"));
        assert_eq!(requests.iter().filter(|r| r.starts_with("POST")).count(), 1);
    }

    #[tokio::test]
    async fn identical_duplicate_ids_are_verified_and_accepted_without_writes() {
        let (url, task) = cloud::test_http::server(vec![
            (200, r#"{"files":[{"id":"a"},{"id":"b"}]}"#.into()),
            (200, "same".into()),
            (200, "same".into()),
        ])
        .await;
        let store = DriveBackend {
            client_id: "mock".into(),
            api_base: url.clone(),
            upload_base: url,
            prefix: "app".into(),
            client: cloud::client().unwrap(),
            token: Mutex::new(drive::Token {
                access_token: "mock".into(),
                refresh_token: "mock".into(),
                expires_at: i64::MAX,
                client_id: "mock".into(),
            }),
            writes: Mutex::new(()),
        };
        assert_eq!(store.get_blob("v2/a").await.unwrap(), b"same");
        assert!(task.await.unwrap().iter().all(|r| r.starts_with("GET ")));
    }
    #[tokio::test]
    async fn conflicting_path_ids_fail_closed_before_reading_or_overwriting() {
        let (url, task) = cloud::test_http::server(vec![
            (200, r#"{"files":[{"id":"a"},{"id":"b"}]}"#.into()),
            (200, "one".into()),
            (200, "two".into()),
        ])
        .await;
        let store = DriveBackend {
            client_id: "mock".into(),
            api_base: url.clone(),
            upload_base: url,
            prefix: "app".into(),
            client: cloud::client().unwrap(),
            token: Mutex::new(drive::Token {
                access_token: "mock-token".into(),
                refresh_token: "mock-refresh".into(),
                expires_at: i64::MAX,
                client_id: "mock".into(),
            }),
            writes: Mutex::new(()),
        };
        assert!(store.get_blob("v2/a").await.is_err());
        assert_eq!(task.await.unwrap().len(), 3);
    }
}

fn escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\'', "\\'")
}
impl DriveBackend {
    pub async fn new(client_id: String, prefix: String) -> Result<Self, SyncError> {
        let prefix = prefix.trim_matches('/').to_string();
        if !prefix.is_empty() {
            normalize_blob_path(&prefix)?;
        }
        let token = drive::refresh(&client_id).await?;
        Ok(Self {
            client_id,
            api_base: "https://www.googleapis.com/drive/v3".into(),
            upload_base: "https://www.googleapis.com/upload/drive/v3".into(),
            prefix,
            client: cloud::client()?,
            token: Mutex::new(token),
            writes: Mutex::new(()),
        })
    }
    async fn send(&self, request: reqwest::RequestBuilder) -> Result<reqwest::Response, SyncError> {
        let mut token = self.token.lock().await;
        if token.expires_at <= chrono::Utc::now().timestamp() + 60 {
            *token = drive::refresh(&self.client_id).await?;
        }
        let request = request
            .bearer_auth(&token.access_token)
            .build()
            .map_err(cloud::transport)?;
        drop(token);
        cloud::send(&self.client, request).await
    }
    fn key(&self, path: &str) -> Result<String, SyncError> {
        let path = normalize_blob_path(path)?;
        Ok(if self.prefix.is_empty() {
            path
        } else {
            format!("{}/{path}", self.prefix)
        })
    }
    async fn files(&self, path: Option<&str>) -> Result<Vec<File>, SyncError> {
        let mut query = "trashed = false".to_owned();
        if let Some(path) = path {
            query.push_str(&format!(
                " and appProperties has {{ key='aerotabPath' and value='{}' }}",
                escape(path)
            ));
        }
        let mut page = None;
        let mut tokens = std::collections::HashSet::new();
        let mut files = std::collections::BTreeMap::new();
        loop {
            let mut request = self.client.get(format!("{}/files", self.api_base)).query(&[
                ("spaces", "appDataFolder"),
                ("q", &query),
                ("fields", "nextPageToken,files(id,size,appProperties)"),
                ("pageSize", "1000"),
            ]);
            if let Some(token) = page {
                request = request.query(&[("pageToken", token)]);
            }
            let result: Page = self
                .send(request)
                .await?
                .json()
                .await
                .map_err(cloud::transport)?;
            for file in result.files {
                files.insert(file.id.clone(), file);
            }
            match result.next {
                Some(next) if !next.is_empty() && tokens.insert(next.clone()) => page = Some(next),
                None => break,
                _ => return Err(cloud::transport("Drive pagination cycle")),
            }
        }
        Ok(files.into_values().collect())
    }
    async fn read_id(&self, id: &str) -> Result<Vec<u8>, SyncError> {
        let response = self
            .send(
                self.client
                    .get(format!("{}/files/{id}", self.api_base))
                    .query(&[("alt", "media")]),
            )
            .await?;
        if !response.status().is_success() {
            return Err(cloud::transport("missing Drive object"));
        }
        Ok(response.bytes().await.map_err(cloud::transport)?.to_vec())
    }
    /// Duplicate IDs may be retried creates. Accept them only after comparing
    /// every byte; conflicting ciphertext is preserved and never arbitrarily selected.
    async fn identical_content(&self, files: &[File]) -> Result<Vec<u8>, SyncError> {
        let first = files
            .first()
            .ok_or_else(|| cloud::transport("missing Drive object"))?;
        let content = self.read_id(&first.id).await?;
        for file in &files[1..] {
            if self.read_id(&file.id).await? != content {
                return Err(SyncError::Transport(
                    "Drive conflicting immutable blob IDs; all versions preserved".into(),
                ));
            }
        }
        Ok(content)
    }
    async fn delete_id(&self, id: &str) -> Result<(), SyncError> {
        self.send(self.client.delete(format!("{}/files/{id}", self.api_base)))
            .await?;
        Ok(())
    }
}
#[async_trait]
impl BlobStore for DriveBackend {
    async fn list_blobs(&self, prefix: &str) -> Result<Vec<BlobMeta>, SyncError> {
        if !prefix.is_empty() {
            normalize_blob_path(prefix.trim_end_matches('/'))?;
        }
        let root = if self.prefix.is_empty() {
            String::new()
        } else {
            format!("{}/", self.prefix)
        };
        let mut result = std::collections::BTreeMap::new();
        for file in self.files(None).await? {
            if let Some(path) = file
                .properties
                .get("aerotabPath")
                .and_then(|p| p.strip_prefix(&root))
            {
                if path.starts_with(prefix)
                    && normalize_blob_path(path).is_ok()
                    && result
                        .insert(
                            path.to_owned(),
                            file.size.and_then(|n| n.parse().ok()).unwrap_or(0),
                        )
                        .is_some()
                {
                    let duplicates = self.files(Some(&self.key(path)?)).await?;
                    self.identical_content(&duplicates).await?;
                }
            }
        }
        Ok(result
            .into_iter()
            .map(|(path, size)| BlobMeta { path, size })
            .collect())
    }
    async fn get_blob(&self, path: &str) -> Result<Vec<u8>, SyncError> {
        let key = self.key(path)?;
        let files = self.files(Some(&key)).await?;
        self.identical_content(&files).await
    }
    async fn put_blob(&self, path: &str, bytes: &[u8]) -> Result<(), SyncError> {
        // Serialize per-store lookup+write to avoid same-client duplicate creates.
        let _guard = self.writes.lock().await;
        let key = self.key(path)?;
        let files = self.files(Some(&key)).await?;
        if path == "v2/snapshots/latest" || path.starts_with("v2/snapshots/") {
            return Err(SyncError::Transport("Drive shared snapshot publication disabled until conditional metadata writes are supported".into()));
        }
        if !files.is_empty() {
            if self.identical_content(&files).await? != bytes {
                return Err(SyncError::Transport("Drive immutable blob already exists with different content; original preserved".into()));
            }
            return Ok(());
        } else {
            // Use a pre-generated Drive ID: a retried POST cannot create a second file.
            let ids: serde_json::Value = self
                .send(
                    self.client
                        .get(format!("{}/files/generateIds", self.api_base))
                        .query(&[
                            ("count", "1"),
                            ("space", "appDataFolder"),
                            ("type", "files"),
                        ]),
                )
                .await?
                .json()
                .await
                .map_err(cloud::transport)?;
            let id = ids["ids"][0]
                .as_str()
                .ok_or_else(|| cloud::transport("Drive ID missing"))?;
            let boundary = format!("aerotab-{}", uuid::Uuid::new_v4());
            let metadata = json!({"id":id,"name":key,"parents":["appDataFolder"],"appProperties":{"aerotabPath":key}});
            let mut body=format!("--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{metadata}\r\n--{boundary}\r\nContent-Type: application/octet-stream\r\n\r\n").into_bytes();
            body.extend_from_slice(bytes);
            body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
            let create = self
                .send(
                    self.client
                        .post(format!("{}/files", self.upload_base))
                        .query(&[("uploadType", "multipart")])
                        .header(
                            "content-type",
                            format!("multipart/related; boundary={boundary}"),
                        )
                        .body(body),
                )
                .await;
            if let Err(error) = create {
                // A successful POST may lose its acknowledgement; the generated ID
                // makes retries idempotent, even when the retry reports HTTP 409.
                if self.read_id(id).await.ok().as_deref() != Some(bytes) {
                    return Err(error);
                }
            }
        }
        // Re-list to converge concurrent creates by different devices. Never remove
        // differing ciphertext; duplicates are retained rather than silently losing data.
        let found = self.files(Some(&key)).await?;
        if self.identical_content(&found).await? != bytes {
            return Err(SyncError::Transport(
                "Drive concurrent immutable blob conflict; all IDs preserved".into(),
            ));
        }
        Ok(())
    }
    async fn delete_blob(&self, path: &str) -> Result<(), SyncError> {
        let _guard = self.writes.lock().await;
        let files = self.files(Some(&self.key(path)?)).await?;
        if !files.is_empty() {
            self.identical_content(&files).await?;
        }
        for file in files {
            self.delete_id(&file.id).await?;
        }
        Ok(())
    }
}
