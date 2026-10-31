//! Path-addressed blob store used by the v2 oplog sync protocol.
//!
//! Unlike [`SyncBackend`](super::SyncBackend) (which is record/group oriented
//! and used by the legacy engine), [`BlobStore`] speaks opaque relative paths
//! so the oplog layout (`v2/devices/...`, `v2/snapshots/...`) can evolve
//! without changing the transport adapters.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::SyncError;

/// Metadata returned by [`BlobStore::list_blobs`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobMeta {
    /// Path relative to the store root, using `/` separators, no leading slash.
    pub path: String,
    pub size: u64,
}

// `async_trait` adds a bare `#[must_use]` to required methods returning boxed futures.
#[allow(clippy::double_must_use)]
#[async_trait]
pub trait BlobStore: Send + Sync {
    /// List blobs whose path starts with `prefix` (implementations may return
    /// nested matches under that prefix). Named distinctly from
    /// [`SyncBackend::list`](super::SyncBackend::list) so both traits can be
    /// implemented on the same type without method ambiguity.
    async fn list_blobs(&self, prefix: &str) -> Result<Vec<BlobMeta>, SyncError>;

    async fn get_blob(&self, path: &str) -> Result<Vec<u8>, SyncError>;

    async fn put_blob(&self, path: &str, bytes: &[u8]) -> Result<(), SyncError>;

    async fn delete_blob(&self, path: &str) -> Result<(), SyncError>;
}

/// Normalise a caller-supplied relative path: strip leading `/`, reject `..`.
pub fn normalize_blob_path(path: &str) -> Result<String, SyncError> {
    let trimmed = path.trim_start_matches('/');
    if trimmed.is_empty() {
        return Err(SyncError::Transport("empty blob path".into()));
    }
    for seg in trimmed.split('/') {
        if seg.is_empty() || seg == "." || seg == ".." {
            return Err(SyncError::Transport(format!(
                "invalid blob path segment in `{path}`"
            )));
        }
    }
    Ok(trimmed.to_string())
}

/// In-memory [`BlobStore`] for unit tests.
#[derive(Default)]
pub struct MemBlobStore {
    // path → bytes
    pub store: std::sync::Mutex<std::collections::BTreeMap<String, Vec<u8>>>,
    /// Counts every successful `get` (for asserting incremental behaviour).
    pub get_count: std::sync::atomic::AtomicUsize,
}

impl MemBlobStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn gets(&self) -> usize {
        self.get_count.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[async_trait]
impl BlobStore for MemBlobStore {
    async fn list_blobs(&self, prefix: &str) -> Result<Vec<BlobMeta>, SyncError> {
        let prefix = prefix.trim_start_matches('/');
        let s = self
            .store
            .lock()
            .map_err(|e| SyncError::Transport(e.to_string()))?;
        Ok(s.iter()
            .filter(|(p, _)| prefix.is_empty() || p.starts_with(prefix))
            .map(|(p, v)| BlobMeta {
                path: p.clone(),
                size: v.len() as u64,
            })
            .collect())
    }

    async fn get_blob(&self, path: &str) -> Result<Vec<u8>, SyncError> {
        let path = normalize_blob_path(path)?;
        self.get_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let s = self
            .store
            .lock()
            .map_err(|e| SyncError::Transport(e.to_string()))?;
        s.get(&path)
            .cloned()
            .ok_or_else(|| SyncError::Transport(format!("missing: {path}")))
    }

    async fn put_blob(&self, path: &str, bytes: &[u8]) -> Result<(), SyncError> {
        let path = normalize_blob_path(path)?;
        let mut s = self
            .store
            .lock()
            .map_err(|e| SyncError::Transport(e.to_string()))?;
        s.insert(path, bytes.to_vec());
        Ok(())
    }

    async fn delete_blob(&self, path: &str) -> Result<(), SyncError> {
        let path = normalize_blob_path(path)?;
        let mut s = self
            .store
            .lock()
            .map_err(|e| SyncError::Transport(e.to_string()))?;
        s.remove(&path);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mem_roundtrip_and_prefix_list() {
        let m = MemBlobStore::new();
        m.put_blob("v2/devices/a/ops/0001.bin", b"one")
            .await
            .unwrap();
        m.put_blob("v2/devices/a/ops/0002.bin", b"two")
            .await
            .unwrap();
        m.put_blob("v2/snapshots/latest", b"ptr").await.unwrap();
        let listed = m.list_blobs("v2/devices/a/ops/").await.unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(
            m.get_blob("v2/devices/a/ops/0001.bin").await.unwrap(),
            b"one"
        );
        assert_eq!(m.gets(), 1);
        m.delete_blob("v2/devices/a/ops/0001.bin").await.unwrap();
        assert_eq!(m.list_blobs("v2/devices/a/ops/").await.unwrap().len(), 1);
    }

    #[test]
    fn rejects_path_traversal() {
        assert!(normalize_blob_path("../x").is_err());
        assert!(normalize_blob_path("a/../b").is_err());
        assert!(normalize_blob_path("").is_err());
        assert_eq!(normalize_blob_path("/a/b").unwrap(), "a/b");
    }
}
