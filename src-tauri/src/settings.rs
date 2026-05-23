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
}

impl SettingsStore {
    /// Opens (or creates) the settings sled tree under `dir`.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, SettingsError> {
        let dir = dir.as_ref();
        std::fs::create_dir_all(dir).map_err(sled::Error::Io)?;
        let db = sled::open(dir.join("settings.sled"))?;
        let tree = db.open_tree("settings")?;
        Ok(Self {
            inner: Arc::new(Inner { tree }),
        })
    }

    pub fn get(&self, key: &str) -> Result<Option<Value>, SettingsError> {
        match self.inner.tree.get(key)? {
            None => Ok(None),
            Some(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        }
    }

    pub fn set(&self, key: &str, value: &Value) -> Result<(), SettingsError> {
        let bytes = serde_json::to_vec(value)?;
        self.inner.tree.insert(key, bytes)?;
        self.inner.tree.flush()?;
        Ok(())
    }

    pub fn remove(&self, key: &str) -> Result<bool, SettingsError> {
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
