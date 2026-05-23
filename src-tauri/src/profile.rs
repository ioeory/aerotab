//! Connection profile store (sled-backed).
//!
//! A profile is a saveable connection blueprint — currently SSH only. This
//! lives outside the [`sync`](crate::sync) layer because:
//!
//! 1. Not every install enables sync (it's opt-in).
//! 2. Profiles change at human-keystroke rate; we don't need a CRDT here.
//! 3. They're scoped to one device; the sync layer (when configured)
//!    handles cross-device propagation by syncing the same bytes.
//!
//! Layout: one `sled::Tree` named `"profiles"` inside the state DB.
//! Key = profile UUID bytes; value = JSON-serialised [`Profile`].

use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sled::Db;
use uuid::Uuid;

use crate::ssh::SshProfile;

const PROFILE_SCHEMA_VERSION: u32 = 1;

fn default_profile_schema_version() -> u32 {
    PROFILE_SCHEMA_VERSION
}

#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    #[error("sled: {0}")]
    Sled(String),
    #[error("json: {0}")]
    Json(String),
    #[error("not found")]
    NotFound,
}

impl From<sled::Error> for ProfileError {
    fn from(e: sled::Error) -> Self {
        ProfileError::Sled(e.to_string())
    }
}

impl From<serde_json::Error> for ProfileError {
    fn from(e: serde_json::Error) -> Self {
        ProfileError::Json(e.to_string())
    }
}

/// Connection profile. Currently only SSH; future variants (Telnet, RDP)
/// can be added without breaking the wire format thanks to `#[serde(tag)]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ProfileKind {
    Ssh { ssh: SshProfile },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    /// Frontend-facing schema marker used for non-breaking metadata
    /// migrations. Older stored profiles default to the current schema.
    #[serde(default = "default_profile_schema_version", rename = "schemaVersion")]
    pub schema_version: u32,
    pub id: Uuid,
    pub name: String,
    /// Optional grouping label for the UI sidebar.
    #[serde(default)]
    pub group: Option<String>,
    /// Searchable labels used by profile picker/sidebar/settings summaries.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Optional icon metadata. The frontend decides how to render built-in,
    /// emoji, file, or data icons.
    #[serde(default)]
    pub icon: Option<ProfileIcon>,
    /// User-pinned profile, sorted first in UI lists.
    #[serde(default)]
    pub favorite: bool,
    #[serde(flatten)]
    pub spec: ProfileKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProfileIcon {
    pub kind: String,
    pub value: String,
}

const TREE_NAME: &str = "profiles";

#[derive(Clone)]
pub struct ProfileStore {
    inner: Arc<Inner>,
}

struct Inner {
    db: Db,
}

impl ProfileStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, ProfileError> {
        let db = sled::open(path.as_ref())?;
        // Touch the tree to make sure it exists.
        let _ = db.open_tree(TREE_NAME)?;
        Ok(Self {
            inner: Arc::new(Inner { db }),
        })
    }

    fn tree(&self) -> Result<sled::Tree, ProfileError> {
        self.inner.db.open_tree(TREE_NAME).map_err(Into::into)
    }

    pub async fn list(&self) -> Result<Vec<Profile>, ProfileError> {
        let me = self.clone();
        tokio::task::spawn_blocking(move || {
            let tree = me.tree()?;
            let mut out = Vec::new();
            for kv in tree.iter() {
                let (_, v) = kv?;
                let p: Profile = serde_json::from_slice(&v)?;
                out.push(p);
            }
            Ok(out)
        })
        .await
        .map_err(|e| ProfileError::Sled(format!("join: {e}")))?
    }

    pub async fn get(&self, id: Uuid) -> Result<Option<Profile>, ProfileError> {
        let me = self.clone();
        tokio::task::spawn_blocking(move || {
            let tree = me.tree()?;
            match tree.get(id.as_bytes())? {
                Some(v) => Ok(Some(serde_json::from_slice(&v)?)),
                None => Ok(None),
            }
        })
        .await
        .map_err(|e| ProfileError::Sled(format!("join: {e}")))?
    }

    pub async fn upsert(&self, profile: Profile) -> Result<(), ProfileError> {
        let me = self.clone();
        tokio::task::spawn_blocking(move || {
            let tree = me.tree()?;
            let bytes = serde_json::to_vec(&profile)?;
            tree.insert(profile.id.as_bytes(), bytes)?;
            tree.flush()?;
            Ok(())
        })
        .await
        .map_err(|e| ProfileError::Sled(format!("join: {e}")))?
    }

    pub async fn delete(&self, id: Uuid) -> Result<bool, ProfileError> {
        let me = self.clone();
        tokio::task::spawn_blocking(move || {
            let tree = me.tree()?;
            let removed = tree.remove(id.as_bytes())?.is_some();
            tree.flush()?;
            Ok(removed)
        })
        .await
        .map_err(|e| ProfileError::Sled(format!("join: {e}")))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ssh::{AuthMethod, SshProfile};

    fn tmp() -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("tabby-profile-{}", Uuid::new_v4()));
        p
    }

    fn ssh_profile(id: Uuid, name: &str) -> Profile {
        Profile {
            schema_version: PROFILE_SCHEMA_VERSION,
            id,
            name: name.into(),
            group: None,
            tags: vec![],
            icon: None,
            favorite: false,
            spec: ProfileKind::Ssh {
                ssh: SshProfile {
                    host: "example.com".into(),
                    port: 22,
                    user: "root".into(),
                    auth: AuthMethod::Password { secret: "x".into() },
                    jump_via: vec![],
                },
            },
        }
    }

    #[tokio::test]
    async fn list_empty() {
        let dir = tmp();
        let s = ProfileStore::open(&dir).unwrap();
        assert!(s.list().await.unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn upsert_list_get_delete() {
        let dir = tmp();
        let s = ProfileStore::open(&dir).unwrap();
        let id = Uuid::new_v4();
        s.upsert(ssh_profile(id, "prod")).await.unwrap();
        let listed = s.list().await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "prod");

        let got = s.get(id).await.unwrap().unwrap();
        assert_eq!(got.name, "prod");

        // Update
        let mut updated = got.clone();
        updated.name = "prod2".into();
        s.upsert(updated).await.unwrap();
        assert_eq!(s.get(id).await.unwrap().unwrap().name, "prod2");

        // Delete
        assert!(s.delete(id).await.unwrap());
        assert!(!s.delete(id).await.unwrap());
        assert!(s.list().await.unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn survives_reopen() {
        let dir = tmp();
        let id = Uuid::new_v4();
        {
            let s = ProfileStore::open(&dir).unwrap();
            s.upsert(ssh_profile(id, "p")).await.unwrap();
        }
        let s = ProfileStore::open(&dir).unwrap();
        assert_eq!(s.list().await.unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn legacy_profile_defaults_new_metadata() {
        let id = Uuid::new_v4();
        let raw = serde_json::json!({
            "id": id,
            "name": "legacy",
            "kind": "ssh",
            "ssh": {
                "host": "example.com",
                "port": 22,
                "user": "root",
                "auth": { "Password": { "secret": "x" } },
                "jump_via": []
            }
        });
        let profile: Profile = serde_json::from_value(raw).unwrap();
        assert_eq!(profile.schema_version, PROFILE_SCHEMA_VERSION);
        assert!(profile.tags.is_empty());
        assert!(profile.icon.is_none());
        assert!(!profile.favorite);
    }
}
