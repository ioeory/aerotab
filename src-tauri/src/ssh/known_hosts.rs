//! Persistent host-key pinning ("known_hosts") for the SSH client.
//!
//! Layout: a JSON file at `<state_dir>/known_hosts.json` with the structure
//!
//! ```json
//! {
//!   "host:port": { "key_b64": "...", "key_type": "ssh-rsa" }
//! }
//! ```
//!
//! Trust model:
//!
//! 1. On first connection, the key is recorded (TOFU).
//! 2. On every subsequent connection, the presented key must match the
//!    pinned value byte-for-byte; mismatches fail with
//!    [`KnownHostsError::Mismatch`].
//! 3. Operators can prune entries via the [`remove`](KnownHosts::remove) API
//!    (exposed over IPC as `ssh.knownHosts.remove`).
//!
//! The file is rewritten atomically (tmp + rename) so a crash mid-write
//! can't leave a half-formed JSON document.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum KnownHostsError {
    #[error("io: {0}")]
    Io(String),
    #[error("json: {0}")]
    Json(String),
    #[error("host key mismatch for {host}: pinned {pinned_kind} != offered {offered_kind}")]
    Mismatch {
        host: String,
        pinned_kind: String,
        offered_kind: String,
    },
}

impl From<std::io::Error> for KnownHostsError {
    fn from(e: std::io::Error) -> Self {
        KnownHostsError::Io(e.to_string())
    }
}
impl From<serde_json::Error> for KnownHostsError {
    fn from(e: serde_json::Error) -> Self {
        KnownHostsError::Json(e.to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PinnedKey {
    pub key_b64: String,
    pub key_type: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct OnDisk {
    #[serde(default)]
    hosts: BTreeMap<String, PinnedKey>,
}

#[derive(Clone)]
pub struct KnownHosts {
    inner: Arc<Inner>,
}

struct Inner {
    path: PathBuf,
    state: Mutex<OnDisk>,
}

/// Outcome of a host-key verification.
#[derive(Debug)]
pub enum Verdict {
    /// First time we've seen this host; key was just pinned.
    Tofu,
    /// Key matches the pinned value. Connection should proceed.
    Match,
}

impl KnownHosts {
    /// Opens the store at `<dir>/known_hosts.json`, creating the directory
    /// if needed. Missing files yield an empty store.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, KnownHostsError> {
        let dir = dir.as_ref();
        std::fs::create_dir_all(dir)?;
        let path = dir.join("known_hosts.json");
        let state = if path.exists() {
            let bytes = std::fs::read(&path)?;
            // Tolerate empty file from an interrupted write.
            if bytes.is_empty() {
                OnDisk::default()
            } else {
                serde_json::from_slice(&bytes)?
            }
        } else {
            OnDisk::default()
        };
        Ok(Self {
            inner: Arc::new(Inner {
                path,
                state: Mutex::new(state),
            }),
        })
    }

    fn save(&self, state: &OnDisk) -> Result<(), KnownHostsError> {
        let bytes = serde_json::to_vec_pretty(state)?;
        let tmp = self.inner.path.with_extension("json.tmp");
        std::fs::write(&tmp, &bytes)?;
        std::fs::rename(&tmp, &self.inner.path)?;
        Ok(())
    }

    /// Verifies (or pins, on first sight) a host key.
    pub fn verify(
        &self,
        host_port: &str,
        offered_key_b64: &str,
        offered_key_type: &str,
    ) -> Result<Verdict, KnownHostsError> {
        let mut state = self.inner.state.lock().unwrap();
        match state.hosts.get(host_port) {
            Some(pinned)
                if pinned.key_b64 == offered_key_b64 && pinned.key_type == offered_key_type =>
            {
                Ok(Verdict::Match)
            }
            Some(pinned) => Err(KnownHostsError::Mismatch {
                host: host_port.to_string(),
                pinned_kind: pinned.key_type.clone(),
                offered_kind: offered_key_type.to_string(),
            }),
            None => {
                state.hosts.insert(
                    host_port.to_string(),
                    PinnedKey {
                        key_b64: offered_key_b64.to_string(),
                        key_type: offered_key_type.to_string(),
                    },
                );
                self.save(&state)?;
                Ok(Verdict::Tofu)
            }
        }
    }

    /// Returns every pinned entry as `(host, key)` pairs.
    pub fn list(&self) -> Vec<(String, PinnedKey)> {
        let state = self.inner.state.lock().unwrap();
        state
            .hosts
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// Removes a single host pin. Returns true if something was removed.
    pub fn remove(&self, host_port: &str) -> Result<bool, KnownHostsError> {
        let mut state = self.inner.state.lock().unwrap();
        let removed = state.hosts.remove(host_port).is_some();
        if removed {
            self.save(&state)?;
        }
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn tmp() -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("tabby-kh-{}", Uuid::new_v4()));
        p
    }

    #[test]
    fn tofu_then_match() {
        let dir = tmp();
        let kh = KnownHosts::open(&dir).unwrap();
        let v = kh.verify("example.com:22", "AAA", "ssh-ed25519").unwrap();
        assert!(matches!(v, Verdict::Tofu));
        let v = kh.verify("example.com:22", "AAA", "ssh-ed25519").unwrap();
        assert!(matches!(v, Verdict::Match));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mismatch_is_an_error() {
        let dir = tmp();
        let kh = KnownHosts::open(&dir).unwrap();
        kh.verify("h:22", "AAA", "ssh-ed25519").unwrap();
        let err = kh.verify("h:22", "BBB", "ssh-ed25519").unwrap_err();
        assert!(matches!(err, KnownHostsError::Mismatch { .. }));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn survives_reopen() {
        let dir = tmp();
        {
            let kh = KnownHosts::open(&dir).unwrap();
            kh.verify("a:22", "K", "ed25519").unwrap();
        }
        let kh = KnownHosts::open(&dir).unwrap();
        let listed = kh.list();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].0, "a:22");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn remove_takes_effect() {
        let dir = tmp();
        let kh = KnownHosts::open(&dir).unwrap();
        kh.verify("a:22", "K", "ed25519").unwrap();
        assert!(kh.remove("a:22").unwrap());
        assert!(!kh.remove("a:22").unwrap());
        assert!(kh.list().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
