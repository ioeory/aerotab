//! WebDAV sync backend.
//!
//! Layout on the server (under `base_url`):
//!
//! ```text
//! <base_url>/<group>/<record-uuid>.bin   # AEAD envelope bytes
//! ```
//!
//! Atomic writes: PUT to `<id>.bin.tmp`, then MOVE to `<id>.bin`. If the MOVE
//! is unsupported by the server we fall back to a direct PUT (some WebDAV
//! servers are not fully compliant).
//!
//! Auth: HTTP Basic only (sufficient for Nextcloud, Apache mod_dav, etc.).

use std::time::Duration;

use async_trait::async_trait;
use reqwest::{header, Client, Method, StatusCode};
use uuid::Uuid;

use crate::sync::{Group, RecordId, SyncBackend, SyncError};

#[derive(Debug, Clone)]
pub struct WebDavBackend {
    base_url: String,
    user: Option<String>,
    password: Option<String>,
    client: Client,
}

impl WebDavBackend {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self::with_auth(base_url, None, None)
    }

    pub fn with_auth(
        base_url: impl Into<String>,
        user: Option<String>,
        password: Option<String>,
    ) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent(concat!("aerotab-core/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("reqwest client build");
        let mut base = base_url.into();
        if !base.ends_with('/') {
            base.push('/');
        }
        Self {
            base_url: base,
            user,
            password,
            client,
        }
    }

    fn group_dir(&self, group: Group) -> String {
        format!("{}{}/", self.base_url, group_segment(group))
    }

    fn record_url(&self, group: Group, id: RecordId) -> String {
        format!("{}{}.bin", self.group_dir(group), id.0)
    }

    fn record_tmp_url(&self, group: Group, id: RecordId) -> String {
        format!("{}{}.bin.tmp", self.group_dir(group), id.0)
    }

    fn auth(&self, rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(u) = &self.user {
            rb.basic_auth(u, self.password.as_deref())
        } else {
            rb
        }
    }

    /// Ensures the group directory exists. Idempotent — `MKCOL` on an
    /// existing collection returns 405 which we treat as success.
    pub async fn ensure_group(&self, group: Group) -> Result<(), SyncError> {
        let url = self.group_dir(group);
        let rb = self
            .client
            .request(Method::from_bytes(b"MKCOL").unwrap(), &url);
        let resp = self.auth(rb).send().await.map_err(transport)?;
        match resp.status().as_u16() {
            201 | 200 | 405 => Ok(()),
            401 | 403 => Err(SyncError::Auth),
            code => Err(SyncError::Transport(format!("MKCOL {url} -> {code}"))),
        }
    }
}

fn transport(e: reqwest::Error) -> SyncError {
    SyncError::Transport(e.to_string())
}

fn group_segment(group: Group) -> &'static str {
    match group {
        Group::Connections => "connections",
        Group::Appearance => "appearance",
        Group::Shortcuts => "shortcuts",
        Group::PluginCfg => "plugincfg",
        Group::Credentials => "credentials",
    }
}

const PROPFIND_BODY: &str = r#"<?xml version="1.0"?>
<d:propfind xmlns:d="DAV:"><d:prop><d:resourcetype/></d:prop></d:propfind>"#;

#[async_trait]
impl SyncBackend for WebDavBackend {
    async fn list(&self, group: Group) -> Result<Vec<RecordId>, SyncError> {
        let url = self.group_dir(group);
        let rb = self
            .client
            .request(Method::from_bytes(b"PROPFIND").unwrap(), &url)
            .header("Depth", "1")
            .header(header::CONTENT_TYPE, "application/xml")
            .body(PROPFIND_BODY);
        let resp = self.auth(rb).send().await.map_err(transport)?;
        match resp.status().as_u16() {
            207 => {}
            404 => return Ok(vec![]),
            401 | 403 => return Err(SyncError::Auth),
            code => return Err(SyncError::Transport(format!("PROPFIND -> {code}"))),
        }
        let body = resp.text().await.map_err(transport)?;
        Ok(parse_propfind_ids(&body))
    }

    async fn get(&self, group: Group, id: RecordId) -> Result<Vec<u8>, SyncError> {
        let url = self.record_url(group, id);
        let resp = self
            .auth(self.client.get(&url))
            .send()
            .await
            .map_err(transport)?;
        match resp.status() {
            StatusCode::OK => Ok(resp.bytes().await.map_err(transport)?.to_vec()),
            StatusCode::NOT_FOUND => Err(SyncError::Transport(format!("missing: {url}"))),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(SyncError::Auth),
            code => Err(SyncError::Transport(format!("GET -> {code}"))),
        }
    }

    async fn put(&self, group: Group, id: RecordId, blob: &[u8]) -> Result<(), SyncError> {
        // best-effort directory creation; ignore "already exists"
        let _ = self.ensure_group(group).await;

        let tmp = self.record_tmp_url(group, id);
        let final_ = self.record_url(group, id);

        let resp = self
            .auth(self.client.put(&tmp))
            .body(blob.to_vec())
            .send()
            .await
            .map_err(transport)?;
        match resp.status().as_u16() {
            200 | 201 | 204 => {}
            401 | 403 => return Err(SyncError::Auth),
            code => return Err(SyncError::Transport(format!("PUT tmp -> {code}"))),
        }

        // MOVE tmp -> final
        let move_rb = self
            .client
            .request(Method::from_bytes(b"MOVE").unwrap(), &tmp)
            .header("Destination", &final_)
            .header("Overwrite", "T");
        let resp = self.auth(move_rb).send().await.map_err(transport)?;
        match resp.status().as_u16() {
            200 | 201 | 204 => Ok(()),
            401 | 403 => Err(SyncError::Auth),
            // Server doesn't support MOVE — fall back to direct PUT.
            405 | 501 => {
                let resp = self
                    .auth(self.client.put(&final_))
                    .body(blob.to_vec())
                    .send()
                    .await
                    .map_err(transport)?;
                match resp.status().as_u16() {
                    200 | 201 | 204 => Ok(()),
                    code => Err(SyncError::Transport(format!("PUT fallback -> {code}"))),
                }
            }
            code => Err(SyncError::Transport(format!("MOVE -> {code}"))),
        }
    }

    async fn delete(&self, group: Group, id: RecordId) -> Result<(), SyncError> {
        let url = self.record_url(group, id);
        let resp = self
            .auth(self.client.delete(&url))
            .send()
            .await
            .map_err(transport)?;
        match resp.status().as_u16() {
            200 | 202 | 204 | 404 => Ok(()),
            401 | 403 => Err(SyncError::Auth),
            code => Err(SyncError::Transport(format!("DELETE -> {code}"))),
        }
    }
}

/// Extract `<id>.bin` UUIDs from a `multistatus` PROPFIND response.
/// Uses a substring scan to avoid pulling in an XML parser.
fn parse_propfind_ids(body: &str) -> Vec<RecordId> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    // crude but adequate: look for ".bin<" segments preceded by a UUID.
    for href_seg in body.split("<d:href>").skip(1) {
        let Some(end) = href_seg.find("</d:href>") else {
            continue;
        };
        let href = &href_seg[..end];
        if let Some(name) = href.rsplit('/').find(|s| s.ends_with(".bin")) {
            let stem = name.trim_end_matches(".bin");
            if let Ok(u) = Uuid::parse_str(stem) {
                if seen.insert(u) {
                    out.push(RecordId(u));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_propfind_extracts_uuid_filenames() {
        let body = r#"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:">
  <d:response><d:href>/dav/connections/</d:href></d:response>
  <d:response><d:href>/dav/connections/11111111-1111-1111-1111-111111111111.bin</d:href></d:response>
  <d:response><d:href>/dav/connections/22222222-2222-2222-2222-222222222222.bin</d:href></d:response>
  <d:response><d:href>/dav/connections/not-a-uuid.bin</d:href></d:response>
  <d:response><d:href>/dav/connections/readme.txt</d:href></d:response>
</d:multistatus>"#;
        let ids = parse_propfind_ids(body);
        assert_eq!(ids.len(), 2);
    }

    #[test]
    fn record_url_construction() {
        let b = WebDavBackend::new("https://dav.example/sync");
        let id = RecordId(uuid::uuid!("11111111-1111-1111-1111-111111111111"));
        assert_eq!(
            b.record_url(Group::Connections, id),
            "https://dav.example/sync/connections/11111111-1111-1111-1111-111111111111.bin"
        );
    }

    #[test]
    fn group_segments_stable() {
        assert_eq!(group_segment(Group::Connections), "connections");
        assert_eq!(group_segment(Group::Credentials), "credentials");
    }
}
