//! Master-password protected secret vault (M10).
//!
//! On-disk layout in the sled DB tree `vault`:
//! - `__verifier`  → small canary blob encrypted with the user's password
//!   using the same envelope format as the sync module.
//! - `<entry_id>`  → individual entry payload encrypted with the same
//!   envelope format. Decrypted value is a JSON
//!   [`VaultEntry`] (label, kind, secret).
//!
//! The vault holds no in-memory plaintext copy of secrets — every read
//! decrypts on demand. Only the master password (zeroized [`Vec<u8>`]) is
//! kept while unlocked.

use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::RwLock;
use uuid::Uuid;
use zeroize::Zeroize;

use crate::sync::crypto::{decrypt, encrypt, CryptoError, KdfParams};

const VERIFIER_KEY: &str = "__verifier";
const VERIFIER_PAYLOAD: &[u8] = b"tabby-vault-v1";

#[derive(Debug, Error)]
pub enum VaultError {
    #[error("sled error: {0}")]
    Sled(#[from] sled::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("crypto error: {0}")]
    Crypto(#[from] CryptoError),
    #[error("vault is locked")]
    Locked,
    #[error("entry not found: {0}")]
    NotFound(String),
    #[error("incorrect master password")]
    BadPassword,
    #[error("vault already initialized — use unlock")]
    AlreadyInitialized,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    Password,
    Note,
    Token,
    Key,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VaultEntry {
    pub id: String,
    pub label: String,
    pub kind: EntryKind,
    #[serde(default)]
    pub username: Option<String>,
    /// Secret value (password / token / private key text / free-form note).
    pub secret: String,
}

/// Public summary returned to the UI (omits the secret).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultEntryMeta {
    pub id: String,
    pub label: String,
    pub kind: EntryKind,
    #[serde(default)]
    pub username: Option<String>,
}

impl From<&VaultEntry> for VaultEntryMeta {
    fn from(e: &VaultEntry) -> Self {
        Self {
            id: e.id.clone(),
            label: e.label.clone(),
            kind: e.kind.clone(),
            username: e.username.clone(),
        }
    }
}

#[derive(Clone)]
pub struct VaultStore {
    inner: Arc<Inner>,
}

struct Inner {
    tree: sled::Tree,
    /// `None` when locked. `Some(password)` when unlocked.
    secret: RwLock<Option<Vec<u8>>>,
    params: KdfParams,
}

impl VaultStore {
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, VaultError> {
        Self::open_with_params(dir, KdfParams::default())
    }

    pub fn open_with_params(dir: impl AsRef<Path>, params: KdfParams) -> Result<Self, VaultError> {
        let dir = dir.as_ref();
        std::fs::create_dir_all(dir).map_err(sled::Error::Io)?;
        let db = sled::open(dir.join("vault.sled"))?;
        let tree = db.open_tree("vault")?;
        Ok(Self {
            inner: Arc::new(Inner {
                tree,
                secret: RwLock::new(None),
                params,
            }),
        })
    }

    pub fn is_initialized(&self) -> Result<bool, VaultError> {
        Ok(self.inner.tree.contains_key(VERIFIER_KEY)?)
    }

    pub async fn is_unlocked(&self) -> bool {
        self.inner.secret.read().await.is_some()
    }

    /// Creates the verifier blob and unlocks the vault. Fails if the vault
    /// has already been initialized.
    pub async fn initialize(&self, password: &[u8]) -> Result<(), VaultError> {
        if self.is_initialized()? {
            return Err(VaultError::AlreadyInitialized);
        }
        let envelope = encrypt(password, VERIFIER_PAYLOAD, self.inner.params)?;
        self.inner.tree.insert(VERIFIER_KEY, envelope)?;
        self.inner.tree.flush()?;
        *self.inner.secret.write().await = Some(password.to_vec());
        Ok(())
    }

    /// Verifies the password against the stored verifier and, on success,
    /// caches it in memory for subsequent get/put calls.
    pub async fn unlock(&self, password: &[u8]) -> Result<(), VaultError> {
        let envelope = self
            .inner
            .tree
            .get(VERIFIER_KEY)?
            .ok_or(VaultError::Locked)?;
        let pt =
            decrypt(password, &envelope, self.inner.params).map_err(|_| VaultError::BadPassword)?;
        if pt != VERIFIER_PAYLOAD {
            return Err(VaultError::BadPassword);
        }
        *self.inner.secret.write().await = Some(password.to_vec());
        Ok(())
    }

    pub async fn lock(&self) {
        let mut g = self.inner.secret.write().await;
        if let Some(mut s) = g.take() {
            s.zeroize();
        }
    }

    /// Returns metadata for every entry (label, kind, id). Requires unlock.
    pub async fn list(&self) -> Result<Vec<VaultEntryMeta>, VaultError> {
        let secret = self.read_secret().await?;
        let mut out = Vec::new();
        for kv in self.inner.tree.iter() {
            let (k, v) = kv?;
            if k.as_ref() == VERIFIER_KEY.as_bytes() {
                continue;
            }
            let pt = decrypt(&secret, &v, self.inner.params)?;
            let entry: VaultEntry = serde_json::from_slice(&pt)?;
            out.push(VaultEntryMeta::from(&entry));
        }
        out.sort_by_key(|e| e.label.to_lowercase());
        Ok(out)
    }

    pub async fn get(&self, id: &str) -> Result<VaultEntry, VaultError> {
        let secret = self.read_secret().await?;
        let raw = self
            .inner
            .tree
            .get(id)?
            .ok_or_else(|| VaultError::NotFound(id.to_string()))?;
        let pt = decrypt(&secret, &raw, self.inner.params)?;
        let entry: VaultEntry = serde_json::from_slice(&pt)?;
        Ok(entry)
    }

    /// Inserts or replaces an entry. Assigns a fresh id if missing.
    pub async fn put(&self, mut entry: VaultEntry) -> Result<VaultEntry, VaultError> {
        let secret = self.read_secret().await?;
        if entry.id.is_empty() {
            entry.id = Uuid::new_v4().to_string();
        }
        let json = serde_json::to_vec(&entry)?;
        let envelope = encrypt(&secret, &json, self.inner.params)?;
        self.inner.tree.insert(entry.id.as_bytes(), envelope)?;
        self.inner.tree.flush()?;
        Ok(entry)
    }

    pub async fn remove(&self, id: &str) -> Result<bool, VaultError> {
        let _ = self.read_secret().await?; // require unlock
        let prev = self.inner.tree.remove(id)?;
        self.inner.tree.flush()?;
        Ok(prev.is_some())
    }

    /// Change master password by re-encrypting every entry + verifier.
    pub async fn change_password(&self, old: &[u8], new: &[u8]) -> Result<(), VaultError> {
        // verify old
        self.unlock(old).await?;
        // re-write every entry
        for kv in self.inner.tree.iter() {
            let (k, v) = kv?;
            let pt = decrypt(old, &v, self.inner.params)?;
            let envelope = encrypt(new, &pt, self.inner.params)?;
            self.inner.tree.insert(k, envelope)?;
        }
        self.inner.tree.flush()?;
        *self.inner.secret.write().await = Some(new.to_vec());
        Ok(())
    }

    async fn read_secret(&self) -> Result<Vec<u8>, VaultError> {
        match self.inner.secret.read().await.clone() {
            Some(s) => Ok(s),
            None => Err(VaultError::Locked),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmpdir() -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("tabby-vault-{}", Uuid::new_v4()));
        p
    }

    fn cheap() -> KdfParams {
        KdfParams::test_cheap()
    }

    #[tokio::test]
    async fn initialize_and_unlock_roundtrip() {
        let v = VaultStore::open_with_params(tmpdir(), cheap()).unwrap();
        assert!(!v.is_initialized().unwrap());
        v.initialize(b"hunter2").await.unwrap();
        assert!(v.is_initialized().unwrap());
        assert!(v.is_unlocked().await);
        v.lock().await;
        assert!(!v.is_unlocked().await);
        v.unlock(b"hunter2").await.unwrap();
        assert!(v.is_unlocked().await);
    }

    #[tokio::test]
    async fn put_get_remove_entry() {
        let v = VaultStore::open_with_params(tmpdir(), cheap()).unwrap();
        v.initialize(b"pw").await.unwrap();
        let entry = VaultEntry {
            id: String::new(),
            label: "github".into(),
            kind: EntryKind::Token,
            username: Some("alice".into()),
            secret: "ghp_secret".into(),
        };
        let saved = v.put(entry).await.unwrap();
        assert!(!saved.id.is_empty());
        let listed = v.list().await.unwrap();
        assert_eq!(listed.len(), 1);
        let read = v.get(&saved.id).await.unwrap();
        assert_eq!(read.secret, "ghp_secret");
        assert!(v.remove(&saved.id).await.unwrap());
        assert!(v.list().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn wrong_password_rejected() {
        let v = VaultStore::open_with_params(tmpdir(), cheap()).unwrap();
        v.initialize(b"correct").await.unwrap();
        v.lock().await;
        let err = v.unlock(b"wrong").await.unwrap_err();
        assert!(matches!(err, VaultError::BadPassword));
    }

    #[tokio::test]
    async fn locked_operations_fail() {
        let v = VaultStore::open_with_params(tmpdir(), cheap()).unwrap();
        v.initialize(b"pw").await.unwrap();
        v.lock().await;
        assert!(matches!(v.list().await, Err(VaultError::Locked)));
    }

    #[tokio::test]
    async fn change_password_rewrites_entries() {
        let v = VaultStore::open_with_params(tmpdir(), cheap()).unwrap();
        v.initialize(b"old").await.unwrap();
        let e = v
            .put(VaultEntry {
                id: String::new(),
                label: "x".into(),
                kind: EntryKind::Password,
                username: None,
                secret: "s".into(),
            })
            .await
            .unwrap();
        v.change_password(b"old", b"new").await.unwrap();
        v.lock().await;
        v.unlock(b"new").await.unwrap();
        assert_eq!(v.get(&e.id).await.unwrap().secret, "s");
    }
}
