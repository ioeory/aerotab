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
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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
    #[error("invalid resource reference: {0}")]
    Reference(String),
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

/// Remote desktop target (RDP or VNC), optionally reached via SSH tunnel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteDesktopSpec {
    pub host: String,
    #[serde(default = "default_rdp_port")]
    pub port: u16,
    /// When set, a local `-L` forward is opened through this SSH profile id.
    #[serde(default)]
    pub ssh_profile_id: Option<Uuid>,
    #[serde(default)]
    pub local_bind_port: u16,
}

fn default_rdp_port() -> u16 {
    3389
}

/// Connection profile variants (`#[serde(tag)]` wire format).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ProfileKind {
    Ssh {
        ssh: SshProfile,
    },
    Rdp {
        rdp: RemoteDesktopSpec,
    },
    Vnc {
        #[serde(rename = "vnc")]
        spec: RemoteDesktopSpec,
    },
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
    /// Optional user note shown in profile details and context menus.
    #[serde(default)]
    pub note: Option<String>,
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
    read_only: bool,
    /// Serialize sled access — concurrent `spawn_blocking` callers can deadlock on Windows.
    op_lock: Mutex<()>,
}

impl ProfileStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, ProfileError> {
        Self::open_with_mode(path, false)
    }

    /// Opens the store read-only (for a second AeroTab process while another holds the lock).
    pub fn open_readonly(path: impl AsRef<Path>) -> Result<Self, ProfileError> {
        Self::open_with_mode(path, true)
    }

    fn open_with_mode(path: impl AsRef<Path>, read_only: bool) -> Result<Self, ProfileError> {
        let open_path = if read_only {
            crate::sled_snapshot::temp_snapshot(path.as_ref(), "profiles")
                .map_err(|e| ProfileError::Sled(format!("profile snapshot: {e}")))?
        } else {
            path.as_ref().to_path_buf()
        };
        let db = sled::open(&open_path)?;
        let _ = db.open_tree(TREE_NAME)?;
        Ok(Self {
            inner: Arc::new(Inner {
                db,
                read_only,
                op_lock: Mutex::new(()),
            }),
        })
    }

    fn ensure_writable(&self) -> Result<(), ProfileError> {
        if self.inner.read_only {
            return Err(ProfileError::Sled(
                "profile store is read-only (another AeroTab instance holds the database lock)"
                    .into(),
            ));
        }
        Ok(())
    }

    fn tree(&self) -> Result<sled::Tree, ProfileError> {
        self.inner.db.open_tree(TREE_NAME).map_err(Into::into)
    }

    fn with_tree<T>(
        &self,
        f: impl FnOnce(&sled::Tree) -> Result<T, ProfileError>,
    ) -> Result<T, ProfileError> {
        let _guard = self
            .inner
            .op_lock
            .lock()
            .map_err(|e| ProfileError::Sled(format!("profile store lock: {e}")))?;
        let tree = self.tree()?;
        f(&tree)
    }

    pub async fn list(&self) -> Result<Vec<Profile>, ProfileError> {
        let me = self.clone();
        const TIMEOUT: Duration = Duration::from_secs(12);
        tokio::time::timeout(TIMEOUT, async {
            tokio::task::spawn_blocking(move || {
                me.with_tree(|tree| {
                    let started = Instant::now();
                    const MAX_SCAN: Duration = Duration::from_secs(10);
                    let mut out = Vec::new();
                    for kv in tree.iter() {
                        if started.elapsed() > MAX_SCAN {
                            tracing::warn!("profile list: scan time limit reached");
                            break;
                        }
                        let (_, v) = match kv {
                            Ok(pair) => pair,
                            Err(e) => {
                                tracing::warn!(error = %e, "profile list: skip corrupt sled key");
                                continue;
                            }
                        };
                        match serde_json::from_slice::<Profile>(&v) {
                            Ok(p) => out.push(p),
                            Err(e) => {
                                tracing::warn!(error = %e, "profile list: skip corrupt profile JSON");
                            }
                        }
                    }
                    Ok(out)
                })
            })
            .await
            .map_err(|e| ProfileError::Sled(format!("join: {e}")))?
        })
        .await
        .map_err(|_| {
            ProfileError::Sled(
                "profile list timed out (close other AeroTab instances or check antivirus locking profiles.sled)".into(),
            )
        })?
    }

    pub async fn get(&self, id: Uuid) -> Result<Option<Profile>, ProfileError> {
        let me = self.clone();
        tokio::task::spawn_blocking(move || {
            me.with_tree(|tree| match tree.get(id.as_bytes())? {
                Some(v) => Ok(Some(serde_json::from_slice(&v)?)),
                None => Ok(None),
            })
        })
        .await
        .map_err(|e| ProfileError::Sled(format!("join: {e}")))?
    }

    pub async fn upsert(&self, profile: Profile) -> Result<(), ProfileError> {
        self.write_profile(profile, true).await
    }

    /// Sync batches may arrive before their referenced bastions. References are
    /// still checked at connection time; never rewrite IDs based on names.
    pub(crate) async fn upsert_synced(&self, profile: Profile) -> Result<(), ProfileError> {
        self.write_profile(profile, false).await
    }

    async fn write_profile(&self, profile: Profile, validate: bool) -> Result<(), ProfileError> {
        self.ensure_writable()?;
        let me = self.clone();
        tokio::task::spawn_blocking(move || {
            me.with_tree(|tree| {
                let mut catalog = read_catalog(tree)?;
                catalog.retain(|p| p.id != profile.id);
                catalog.push(profile.clone());
                if validate {
                    if let ProfileKind::Ssh { ssh } = &profile.spec {
                        resolve_jump_references(ssh, &catalog, &mut vec![profile.id])?;
                    }
                }
                let bytes = serde_json::to_vec(&profile)?;
                tree.insert(profile.id.as_bytes(), bytes)?;
                me.inner
                    .db
                    .open_tree("profile_deletions")?
                    .remove(profile.id.as_bytes())?;
                tree.flush()?;
                Ok(())
            })
        })
        .await
        .map_err(|e| ProfileError::Sled(format!("join: {e}")))?
    }

    pub async fn delete(&self, id: Uuid) -> Result<bool, ProfileError> {
        self.ensure_writable()?;
        let me = self.clone();
        tokio::task::spawn_blocking(move || {
            me.with_tree(|tree| {
                let dependencies = profile_dependents(&read_catalog(tree)?, id);
                if !dependencies.is_empty() {
                    return Err(ProfileError::Reference(format!(
                        "profile {id} is used by {}",
                        dependencies.join(", ")
                    )));
                }
                let removed = tree.remove(id.as_bytes())?.is_some();
                if removed {
                    me.inner
                        .db
                        .open_tree("profile_deletions")?
                        .insert(id.as_bytes(), &[])?;
                    me.inner.db.flush()?;
                }
                tree.flush()?;
                Ok(removed)
            })
        })
        .await
        .map_err(|e| ProfileError::Sled(format!("join: {e}")))?
    }
}

fn read_catalog(tree: &sled::Tree) -> Result<Vec<Profile>, ProfileError> {
    tree.iter()
        .map(|row| {
            let (_, bytes) = row?;
            serde_json::from_slice(&bytes).map_err(Into::into)
        })
        .collect()
}

fn ssh_references(ssh: &SshProfile, id: Uuid) -> bool {
    ssh.jump_profile_ids.contains(&id) || ssh.jump_via.iter().any(|hop| ssh_references(hop, id))
}

pub fn profile_dependents(profiles: &[Profile], id: Uuid) -> Vec<String> {
    profiles
        .iter()
        .filter(|p| p.id != id)
        .filter(|p| match &p.spec {
            ProfileKind::Ssh { ssh } => ssh_references(ssh, id),
            ProfileKind::Rdp { rdp } => rdp.ssh_profile_id == Some(id),
            ProfileKind::Vnc { spec } => spec.ssh_profile_id == Some(id),
        })
        .map(|p| format!("{} ({})", p.name, p.id))
        .collect()
}

pub fn vault_dependents(profiles: &[Profile], id: &str) -> Vec<String> {
    fn uses(ssh: &SshProfile, id: &str) -> bool {
        matches!(&ssh.auth, crate::ssh::AuthMethod::VaultRef { entry_id, passphrase_entry_id }
            if entry_id == id || passphrase_entry_id.as_deref() == Some(id))
            || ssh.jump_via.iter().any(|hop| uses(hop, id))
    }
    profiles
        .iter()
        .filter(|p| matches!(&p.spec, ProfileKind::Ssh { ssh } if uses(ssh, id)))
        .map(|p| format!("{} ({})", p.name, p.id))
        .collect()
}

/// Resolve only explicit IDs. Legacy inline snapshots are retained, never name-matched.
pub fn resolve_jump_references(
    ssh: &SshProfile,
    profiles: &[Profile],
    path: &mut Vec<Uuid>,
) -> Result<SshProfile, ProfileError> {
    if path.len() > 32 {
        return Err(ProfileError::Reference(
            "jump chain exceeds 32 levels".into(),
        ));
    }
    if !ssh.jump_profile_ids.is_empty() && !ssh.jump_via.is_empty() {
        return Err(ProfileError::Reference(
            "jump_profile_ids and jump_via are mutually exclusive".into(),
        ));
    }
    let mut resolved = ssh.clone();
    resolved.jump_profile_ids.clear();
    resolved.jump_via.clear();
    for id in &ssh.jump_profile_ids {
        if path.contains(id) {
            return Err(ProfileError::Reference(format!("jump cycle at {id}")));
        }
        let p = profiles
            .iter()
            .find(|p| p.id == *id)
            .ok_or_else(|| ProfileError::Reference(format!("missing jump profile {id}")))?;
        let ProfileKind::Ssh { ssh: hop } = &p.spec else {
            return Err(ProfileError::Reference(format!(
                "jump profile {id} is not SSH"
            )));
        };
        path.push(*id);
        let hop = resolve_jump_references(hop, profiles, path)?;
        path.pop();
        // The transport consumes a flat left-to-right chain.
        resolved.jump_via.extend(hop.jump_via.clone());
        let mut hop = hop;
        hop.jump_via.clear();
        resolved.jump_via.push(hop);
    }
    for hop in &ssh.jump_via {
        resolved
            .jump_via
            .push(resolve_jump_references(hop, profiles, path)?);
    }
    Ok(resolved)
}

impl ProfileStore {
    pub async fn explicit_deletions(&self) -> Result<Vec<Uuid>, ProfileError> {
        let tree = self.inner.db.open_tree("profile_deletions")?;
        tree.iter()
            .map(|row| {
                let (key, _) = row?;
                Uuid::from_slice(&key).map_err(|e| ProfileError::Json(e.to_string()))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ssh::{AuthMethod, SshProfile};

    fn tmp() -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("aerotab-profile-{}", Uuid::new_v4()));
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
            note: None,
            spec: ProfileKind::Ssh {
                ssh: SshProfile {
                    host: "example.com".into(),
                    port: 22,
                    user: "root".into(),
                    auth: AuthMethod::Password { secret: "x".into() },
                    jump_via: vec![],
                    jump_profile_ids: vec![],
                    proxy_id: None,
                    resolved_proxy: None,
                    inherit: vec![],
                },
            },
        }
    }

    fn with_refs(mut profile: Profile, ids: &[Uuid]) -> Profile {
        let ProfileKind::Ssh { ssh } = &mut profile.spec else {
            unreachable!()
        };
        let mut raw = serde_json::to_value(&*ssh).unwrap();
        raw["jump_profile_ids"] = serde_json::to_value(ids).unwrap();
        *ssh = serde_json::from_value(raw).unwrap();
        profile
    }

    #[tokio::test]
    async fn stable_jump_reference_blocks_deletion() {
        let s = ProfileStore::open(tmp()).unwrap();
        let jump = ssh_profile(Uuid::new_v4(), "jump");
        s.upsert(jump.clone()).await.unwrap();
        let target = with_refs(ssh_profile(Uuid::new_v4(), "target"), &[jump.id]);
        s.upsert(target).await.unwrap();
        assert!(
            s.delete(jump.id).await.is_err(),
            "referenced jump must not be deleted"
        );
    }

    #[tokio::test]
    async fn stable_jump_references_reject_missing_and_cycles() {
        let s = ProfileStore::open(tmp()).unwrap();
        let a = ssh_profile(Uuid::new_v4(), "a");
        let b = ssh_profile(Uuid::new_v4(), "b");
        assert!(
            s.upsert(with_refs(a.clone(), &[b.id])).await.is_err(),
            "missing jump must fail"
        );
        s.upsert(b.clone()).await.unwrap();
        s.upsert(with_refs(a.clone(), &[b.id])).await.unwrap();
        assert!(
            s.upsert(with_refs(b, &[a.id])).await.is_err(),
            "cycle must fail"
        );
    }

    #[test]
    fn stable_jump_resolution_uses_latest_profile_and_preserves_snapshots() {
        let mut jump = ssh_profile(Uuid::new_v4(), "renamed");
        let ProfileKind::Ssh { ssh } = &mut jump.spec else {
            unreachable!()
        };
        ssh.host = "latest.example".into();
        let target = with_refs(ssh_profile(Uuid::new_v4(), "target"), &[jump.id]);
        let ProfileKind::Ssh { ssh } = target.spec else {
            unreachable!()
        };
        let resolved = resolve_jump_references(&ssh, &[jump], &mut vec![]).unwrap();
        assert_eq!(resolved.jump_via[0].host, "latest.example");
        assert!(resolved.jump_profile_ids.is_empty());
        let mut legacy = resolved;
        legacy.jump_via[0].host = "snapshot.example".into();
        assert_eq!(
            resolve_jump_references(&legacy, &[], &mut vec![])
                .unwrap()
                .jump_via[0]
                .host,
            "snapshot.example"
        );
        legacy.jump_profile_ids.push(Uuid::new_v4());
        assert!(resolve_jump_references(&legacy, &[], &mut vec![]).is_err());
    }

    #[test]
    fn vault_dependencies_include_passphrase_and_legacy_hops() {
        let mut p = ssh_profile(Uuid::new_v4(), "target");
        let ProfileKind::Ssh { ssh } = &mut p.spec else {
            unreachable!()
        };
        let mut hop = ssh.clone();
        hop.auth = AuthMethod::VaultRef {
            entry_id: "key".into(),
            passphrase_entry_id: Some("passphrase".into()),
        };
        ssh.jump_via.push(hop);
        assert_eq!(vault_dependents(&[p.clone()], "key").len(), 1);
        assert_eq!(vault_dependents(&[p.clone()], "passphrase").len(), 1);
        assert!(vault_dependents(&[p], "unrelated").is_empty());
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
