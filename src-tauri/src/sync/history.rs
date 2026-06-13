//! Persisted sync run log (settings key `syncHistory`).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::settings::{SettingsError, SettingsStore};
use crate::sync::{Group, SyncStats};

const SETTINGS_KEY: &str = "syncHistory";
const MAX_ENTRIES: usize = 200;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncHistoryEntry {
    pub id: Uuid,
    pub at_ms: i64,
    /// `manual` or `auto`.
    pub trigger: String,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub groups: Vec<String>,
    pub pushed: usize,
    pub pulled: usize,
    pub merged: usize,
    pub unchanged: usize,
}

pub fn load(store: &SettingsStore) -> Result<Vec<SyncHistoryEntry>, SettingsError> {
    match store.get(SETTINGS_KEY)? {
        Some(v) => {
            let list: Vec<SyncHistoryEntry> = serde_json::from_value(v)?;
            Ok(list)
        }
        None => Ok(Vec::new()),
    }
}

pub fn append_success(
    store: &SettingsStore,
    trigger: &str,
    groups: &[Group],
    results: &[(Group, SyncStats)],
) -> Result<(), SettingsError> {
    append_success_at(store, trigger, groups, results, now_ms())
}

pub fn append_success_at(
    store: &SettingsStore,
    trigger: &str,
    groups: &[Group],
    results: &[(Group, SyncStats)],
    at_ms: i64,
) -> Result<(), SettingsError> {
    let mut pushed = 0usize;
    let mut pulled = 0usize;
    let mut merged = 0usize;
    let mut unchanged = 0usize;
    for (_, s) in results {
        pushed += s.pushed;
        pulled += s.pulled;
        merged += s.merged;
        unchanged += s.unchanged;
    }
    append(
        store,
        SyncHistoryEntry {
            id: Uuid::new_v4(),
            at_ms,
            trigger: trigger.into(),
            ok: true,
            error: None,
            groups: groups.iter().map(|g| format!("{g:?}")).collect(),
            pushed,
            pulled,
            merged,
            unchanged,
        },
    )
}

pub fn latest_success_ms(store: &SettingsStore) -> Result<Option<i64>, SettingsError> {
    Ok(load(store)?
        .into_iter()
        .find(|entry| entry.ok)
        .map(|entry| entry.at_ms))
}

pub fn append_failure(
    store: &SettingsStore,
    trigger: &str,
    groups: &[Group],
    error: String,
) -> Result<(), SettingsError> {
    append(
        store,
        SyncHistoryEntry {
            id: Uuid::new_v4(),
            at_ms: now_ms(),
            trigger: trigger.into(),
            ok: false,
            error: Some(error),
            groups: groups.iter().map(|g| format!("{g:?}")).collect(),
            pushed: 0,
            pulled: 0,
            merged: 0,
            unchanged: 0,
        },
    )
}

pub fn clear(store: &SettingsStore) -> Result<(), SettingsError> {
    store.set(SETTINGS_KEY, &serde_json::json!([]))
}

fn append(store: &SettingsStore, entry: SyncHistoryEntry) -> Result<(), SettingsError> {
    let mut list = load(store)?;
    list.insert(0, entry);
    if list.len() > MAX_ENTRIES {
        list.truncate(MAX_ENTRIES);
    }
    store.set(SETTINGS_KEY, &serde_json::to_value(&list)?)
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
