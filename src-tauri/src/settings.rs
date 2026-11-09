//! Application settings, persisted to a `sled` tree.
//!
//! The store is intentionally schema-less: keys are short dotted strings,
//! values are arbitrary `serde_json::Value`s. The UI is the source of truth
//! for what a setting "means" — we just provide durable cross-restart KV.

use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SettingsError {
    #[error("sled error: {0}")]
    Sled(#[from] sled::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid settings: {0}")]
    Invalid(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SettingEntry {
    pub key: String,
    pub value: Value,
}

#[derive(Clone)]
pub struct SettingsStore {
    inner: Arc<Inner>,
}

struct Inner {
    tree: sled::Tree,
    /// Device-local metadata (sync run log, cursors, …). Never returned by
    /// [`SettingsStore::all`], so it can never leak into config sync.
    local: sled::Tree,
    read_only: bool,
}

/// Keys that historically lived in the `settings` tree but are device-local
/// state; they are moved into the `local` tree on open.
const LOCAL_MIGRATED_KEYS: &[&str] = &["syncHistory"];

impl SettingsStore {
    /// Opens (or creates) the settings sled tree under `dir`.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, SettingsError> {
        Self::open_with_mode(dir, false)
    }

    /// Read-only open when another process holds the settings DB lock.
    pub fn open_readonly(dir: impl AsRef<Path>) -> Result<Self, SettingsError> {
        Self::open_with_mode(dir, true)
    }

    fn open_with_mode(dir: impl AsRef<Path>, read_only: bool) -> Result<Self, SettingsError> {
        let dir = dir.as_ref();
        if !read_only {
            std::fs::create_dir_all(dir).map_err(sled::Error::Io)?;
        }
        let settings_path = dir.join("settings.sled");
        let open_path = if read_only {
            crate::sled_snapshot::temp_snapshot(&settings_path, "settings")
                .map_err(|e| SettingsError::Sled(sled::Error::Io(e)))?
        } else {
            settings_path
        };
        let db = sled::open(&open_path)?;
        let tree = db.open_tree("settings")?;
        let local = db.open_tree("local")?;
        if !read_only {
            for key in LOCAL_MIGRATED_KEYS {
                if let Some(bytes) = tree.get(key)? {
                    if local.get(key)?.is_none() {
                        local.insert(*key, bytes)?;
                    }
                    tree.remove(key)?;
                }
            }
            tree.flush()?;
            local.flush()?;
        }
        Ok(Self {
            inner: Arc::new(Inner {
                tree,
                local,
                read_only,
            }),
        })
    }

    /// Reads a device-local value (not part of the syncable settings set).
    pub fn local_get(&self, key: &str) -> Result<Option<Value>, SettingsError> {
        match self.inner.local.get(key)? {
            None => Ok(None),
            Some(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        }
    }

    /// Writes a device-local value (not part of the syncable settings set).
    pub fn local_set(&self, key: &str, value: &Value) -> Result<(), SettingsError> {
        if self.inner.read_only {
            return Err(SettingsError::Sled(sled::Error::Unsupported(
                "settings store is read-only (another AeroTab instance holds the database lock)"
                    .into(),
            )));
        }
        let bytes = serde_json::to_vec(value)?;
        self.inner.local.insert(key, bytes)?;
        self.inner.local.flush()?;
        Ok(())
    }

    /// Atomically persist identity and protocol progress in the local tree.
    pub fn local_set_batch(&self, entries: &[(&str, Value)]) -> Result<(), SettingsError> {
        if self.inner.read_only {
            return Err(SettingsError::Sled(sled::Error::Unsupported(
                "settings store is read-only".into(),
            )));
        }
        let mut batch = sled::Batch::default();
        for (key, value) in entries {
            batch.insert(key.as_bytes(), serde_json::to_vec(value)?);
        }
        self.inner.local.apply_batch(batch)?;
        self.inner.local.flush()?;
        Ok(())
    }

    pub fn get(&self, key: &str) -> Result<Option<Value>, SettingsError> {
        match self.inner.tree.get(key)? {
            None => Ok(None),
            Some(bytes) => {
                let value = serde_json::from_slice(&bytes)?;
                if key == crate::profile_inheritance::SETTINGS_KEY {
                    crate::profile_inheritance::parse_defaults(&value)
                        .map_err(SettingsError::Invalid)?;
                }
                Ok(Some(value))
            }
        }
    }

    pub fn set(&self, key: &str, value: &Value) -> Result<(), SettingsError> {
        if key == crate::profile_inheritance::SETTINGS_KEY {
            crate::profile_inheritance::parse_defaults(value).map_err(SettingsError::Invalid)?;
        }
        if self.inner.read_only {
            return Err(SettingsError::Sled(sled::Error::Unsupported(
                "settings store is read-only (another AeroTab instance holds the database lock)"
                    .into(),
            )));
        }
        let bytes = serde_json::to_vec(value)?;
        self.inner.tree.insert(key, bytes)?;
        self.inner.tree.flush()?;
        Ok(())
    }

    pub fn is_read_only(&self) -> bool {
        self.inner.read_only
    }

    pub fn remove(&self, key: &str) -> Result<bool, SettingsError> {
        if self.inner.read_only {
            return Err(SettingsError::Sled(sled::Error::Unsupported(
                "settings store is read-only (another AeroTab instance holds the database lock)"
                    .into(),
            )));
        }
        let prev = self.inner.tree.remove(key)?;
        self.inner.tree.flush()?;
        Ok(prev.is_some())
    }

    pub fn all(&self) -> Result<Vec<SettingEntry>, SettingsError> {
        let mut out = Vec::new();
        for kv in self.inner.tree.iter() {
            let (k, v) = kv?;
            let key = String::from_utf8(k.to_vec())
                .map_err(|e| sled::Error::Unsupported(e.to_string()))?;
            let value: Value = serde_json::from_slice(&v)?;
            if key == crate::profile_inheritance::SETTINGS_KEY {
                crate::profile_inheritance::parse_defaults(&value)
                    .map_err(SettingsError::Invalid)?;
            }
            out.push(SettingEntry { key, value });
        }
        Ok(out)
    }

    /// Removes every key from the store.
    pub fn reset(&self) -> Result<(), SettingsError> {
        self.inner.tree.clear()?;
        self.inner.tree.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;
    use uuid::Uuid;

    fn tmpdir() -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("tabby-settings-test-{}", Uuid::new_v4()));
        p
    }

    #[test]
    fn group_defaults_reject_plaintext_and_invalid_values() {
        let dir = tmpdir();
        let store = SettingsStore::open(&dir).unwrap();
        for value in [
            json!({"team":{"auth":{"Password":{"secret":"secret"}}}}),
            json!({"team":{"auth":{"PublicKey":{"key_path":"key","passphrase":"secret"}}}}),
            json!({"team":{"port":0}}),
            json!({"team/":{"user":"u"}}),
            json!({"team":{"password":"secret"}}),
            json!({"team":{"auth":null}}),
            json!([]),
        ] {
            assert!(
                store.set("profile.groupDefaults", &value).is_err(),
                "accepted {value}"
            );
        }
        assert!(store
            .set(
                "profile.groupDefaults",
                &json!({"team":{"auth":"Agent","proxy_id":null,"jump_profile_ids":[]}})
            )
            .is_ok());
    }

    #[test]
    fn unsafe_legacy_group_defaults_fail_closed_on_read_and_export() {
        let store = SettingsStore::open(tmpdir()).unwrap();
        let unsafe_value = json!({"team":{"auth":{"Password":{"secret":"secret"}}}});
        store
            .inner
            .tree
            .insert(
                crate::profile_inheritance::SETTINGS_KEY,
                serde_json::to_vec(&unsafe_value).unwrap(),
            )
            .unwrap();
        assert!(store.get(crate::profile_inheritance::SETTINGS_KEY).is_err());
        assert!(store.all().is_err());
    }

    #[test]
    fn roundtrip_get_set() {
        let dir = tmpdir();
        let s = SettingsStore::open(&dir).unwrap();
        assert!(s.get("theme").unwrap().is_none());
        s.set("theme", &json!("tokyo-night")).unwrap();
        assert_eq!(s.get("theme").unwrap().unwrap(), json!("tokyo-night"));
    }

    #[test]
    fn all_lists_every_entry() {
        let dir = tmpdir();
        let s = SettingsStore::open(&dir).unwrap();
        s.set("a", &json!(1)).unwrap();
        s.set("b", &json!("x")).unwrap();
        let mut keys: Vec<_> = s.all().unwrap().into_iter().map(|e| e.key).collect();
        keys.sort();
        assert_eq!(keys, vec!["a", "b"]);
    }

    #[test]
    fn reset_clears_everything() {
        let dir = tmpdir();
        let s = SettingsStore::open(&dir).unwrap();
        s.set("a", &json!(1)).unwrap();
        s.reset().unwrap();
        assert!(s.all().unwrap().is_empty());
    }

    #[test]
    fn remove_returns_presence() {
        let dir = tmpdir();
        let s = SettingsStore::open(&dir).unwrap();
        assert!(!s.remove("nope").unwrap());
        s.set("a", &json!(1)).unwrap();
        assert!(s.remove("a").unwrap());
        assert!(s.get("a").unwrap().is_none());
    }

    #[test]
    fn local_tree_is_separate_and_migrates_legacy_keys() {
        let dir = tmpdir();
        {
            let s = SettingsStore::open(&dir).unwrap();
            // Simulate a pre-migration store with syncHistory in the settings tree.
            s.inner
                .tree
                .insert("syncHistory", serde_json::to_vec(&json!([1])).unwrap())
                .unwrap();
            s.set("theme", &json!("x")).unwrap();
        }
        let s = SettingsStore::open(&dir).unwrap();
        assert!(s.get("syncHistory").unwrap().is_none());
        assert_eq!(s.local_get("syncHistory").unwrap().unwrap(), json!([1]));
        let keys: Vec<_> = s.all().unwrap().into_iter().map(|e| e.key).collect();
        assert_eq!(keys, vec!["theme"]);
        s.local_set("cursor", &json!(5)).unwrap();
        assert_eq!(s.all().unwrap().len(), 1);
    }

    #[test]
    fn survives_reopen() {
        let dir = tmpdir();
        {
            let s = SettingsStore::open(&dir).unwrap();
            s.set("color", &json!("#7aa2f7")).unwrap();
        }
        let s2 = SettingsStore::open(&dir).unwrap();
        assert_eq!(s2.get("color").unwrap().unwrap(), json!("#7aa2f7"));
    }
}
