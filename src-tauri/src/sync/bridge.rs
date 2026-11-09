//! Export local stores into the sync engine before `sync.now`, and apply pulled
//! records back into Profile / Settings / Vault after reconciliation.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::plugins::wasm_host::PluginInfo;
use crate::profile::{Profile, ProfileStore};
use crate::secret;
use crate::settings::{SettingEntry, SettingsStore};
use crate::sync::{Group, RecordId, SyncEngine, SyncError};
use crate::vault::{VaultEntry, VaultError, VaultStore};

const SCHEMA: u32 = 1;

/// Legacy (schema 1, pre-0.2.20) record ids: one bundle per group holding
/// every settings entry. Still *read* once for migration; never written.
fn bundle_appearance() -> RecordId {
    RecordId(Uuid::parse_str("a0000001-0001-4000-8000-000000000001").unwrap())
}
fn bundle_shortcuts() -> RecordId {
    RecordId(Uuid::parse_str("a0000001-0001-4000-8000-000000000002").unwrap())
}
fn bundle_plugincfg() -> RecordId {
    RecordId(Uuid::parse_str("a0000001-0001-4000-8000-000000000003").unwrap())
}

/// Namespace for per-key settings records: `uuid_v5(NS, "settings:<key>")`.
const SETTINGS_RECORD_NS: Uuid = Uuid::from_bytes([
    0x6f, 0x3a, 0x9c, 0x1e, 0x55, 0x1b, 0x4d, 0x2a, 0x9a, 0x7e, 0x21, 0x4e, 0x8c, 0x0f, 0xb3, 0x77,
]);

/// Record id for the synced WASM plugin list (PluginCfg group).
fn plugin_list_record() -> RecordId {
    RecordId(Uuid::new_v5(&SETTINGS_RECORD_NS, b"plugins:list"))
}

pub fn settings_record_id(key: &str) -> RecordId {
    RecordId(Uuid::new_v5(
        &SETTINGS_RECORD_NS,
        format!("settings:{key}").as_bytes(),
    ))
}

/// Settings keys that must never leave the device. Any key listed here (or
/// prefixed by an entry ending in `.`) is skipped on export and ignored on
/// import. Keeping volatile state out of the sync set is what prevents every
/// cycle from producing a new upload/commit.
pub const LOCAL_ONLY_SETTINGS_KEYS: &[&str] = &[
    // Engine config + backend secrets; importing a redacted copy would wipe
    // local credentials after the first `sync.now`.
    "sync",
    // MCP listening address, TLS paths, roots, and grants are device-local security policy.
    "mcp",
    "mcp.",
    "ai",
    "ai.",
    // Legacy location of the run log (now in the `local` tree).
    "syncHistory",
    // Per-device session state.
    "openTabs",
    "notes",
    "notes.",
    "recentProfiles",
    // Local consent / diagnostics.
    "sftp.directAgentConsent",
    "diagnostics.",
];

/// Sub-fields of the `window` setting that describe transient per-device UI
/// state rather than preferences.
const LOCAL_ONLY_WINDOW_FIELDS: &[&str] = &["sidebarVisible"];

pub fn is_local_only_key(key: &str) -> bool {
    LOCAL_ONLY_SETTINGS_KEYS.iter().any(|k| {
        if let Some(prefix) = k.strip_suffix('.') {
            key == prefix || key.starts_with(k)
        } else {
            key == *k
        }
    })
}

/// Hotkeys live in the Shortcuts group; `plugin.*` keys in PluginCfg;
/// everything else syncable is Appearance.
fn settings_group_for_key(key: &str) -> Option<Group> {
    if is_local_only_key(key) {
        return None;
    }
    if key == "hotkeys" {
        return Some(Group::Shortcuts);
    }
    if key.starts_with("plugin.") {
        return Some(Group::PluginCfg);
    }
    Some(Group::Appearance)
}

/// One settings key per sync record.
#[derive(Serialize, Deserialize)]
struct SettingsKeyRecord {
    schema: u32,
    key: String,
    value: Value,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct BridgeNotes {
    pub credentials_skipped_locked: bool,
    pub credentials_skipped_uninitialized: bool,
    pub notes_paused_dirty: bool,
}

const DEFAULT_VAULT_KEYRING_ACCOUNT: &str = "sync.vault";

/// Keyring account used to auto-unlock the vault before credential sync.
pub fn vault_keyring_account(settings: Option<&SettingsStore>) -> String {
    let Some(store) = settings else {
        return DEFAULT_VAULT_KEYRING_ACCOUNT.into();
    };
    let Ok(Some(value)) = store.get("sync") else {
        return DEFAULT_VAULT_KEYRING_ACCOUNT.into();
    };
    value
        .get("vaultKeyringAccount")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(DEFAULT_VAULT_KEYRING_ACCOUNT)
        .to_string()
}

/// Unlock the vault from an explicit password and/or OS keyring (for sync / auto-sync).
pub async fn try_unlock_vault_for_sync(
    vault: &VaultStore,
    settings: Option<&SettingsStore>,
    password: Option<&str>,
    keyring_account_override: Option<&str>,
) -> Result<bool, VaultError> {
    if !vault.is_initialized()? {
        return Ok(false);
    }
    if vault.is_unlocked().await {
        return Ok(true);
    }
    if let Some(pw) = password.filter(|s| !s.is_empty()) {
        vault.unlock(pw.as_bytes()).await?;
        return Ok(true);
    }
    let account = keyring_account_override
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| vault_keyring_account(settings));
    if let Ok(pw) = secret::get_master(Some(&account)) {
        vault.unlock(pw.as_bytes()).await?;
        return Ok(true);
    }
    Ok(false)
}

#[derive(Serialize, Deserialize)]
struct SettingsBundle {
    schema: u32,
    entries: Vec<SettingEntry>,
}

#[derive(Serialize, Deserialize)]
struct PluginCfgBundle {
    schema: u32,
    plugins: Vec<PluginInfo>,
}

/// Push local state into the sync engine for the selected groups.
pub async fn export_locals(
    profiles: &ProfileStore,
    settings: &SettingsStore,
    vault: Option<&VaultStore>,
    plugins: &[PluginInfo],
    engine: &SyncEngine,
    groups: &[Group],
) -> Result<BridgeNotes, SyncError> {
    let mut notes = BridgeNotes::default();
    for &group in groups {
        match group {
            Group::Notes => {} // NotesStore is coordinated separately with a revision guard.
            Group::Connections => export_connections(profiles, engine).await?,
            Group::Appearance => export_appearance(settings, engine).await?,
            Group::Shortcuts => export_shortcuts(settings, engine).await?,
            Group::PluginCfg => export_plugin_cfg(settings, plugins, engine).await?,
            Group::Credentials => {
                export_credentials(vault, engine, &mut notes).await?;
            }
        }
    }
    Ok(notes)
}

/// Apply reconciled sync records back into local stores.
pub async fn import_locals(
    profiles: &ProfileStore,
    settings: &SettingsStore,
    vault: Option<&VaultStore>,
    engine: &SyncEngine,
    groups: &[Group],
) -> Result<(), SyncError> {
    for &group in groups {
        match group {
            Group::Notes => {}
            Group::Connections => import_connections(profiles, engine).await?,
            Group::Appearance => import_appearance(settings, engine).await?,
            Group::Shortcuts => import_shortcuts(settings, engine).await?,
            Group::PluginCfg => import_plugin_cfg(settings, engine).await?,
            Group::Credentials => import_credentials(vault, engine).await?,
        }
    }
    Ok(())
}

pub async fn export_notes(
    store: &crate::notes::NotesStore,
    engine: &SyncEngine,
) -> Result<u64, SyncError> {
    export_notes_inner(store, engine, true).await
}
pub async fn preview_notes(
    store: &crate::notes::NotesStore,
    engine: &SyncEngine,
) -> Result<u64, SyncError> {
    export_notes_inner(store, engine, false).await
}
async fn export_notes_inner(
    store: &crate::notes::NotesStore,
    engine: &SyncEngine,
    persist: bool,
) -> Result<u64, SyncError> {
    let (entities, deleted, revision) = store.sync_snapshot().map_err(store_err)?;
    let mut baseline = store.export_baseline().map_err(store_err)?;
    for id in deleted {
        if baseline.get(&id).is_some_and(|bytes| bytes.is_empty())
            && engine
                .snapshot_group(Group::Notes)
                .await
                .iter()
                .any(|(rid, _)| *rid == crate::notes::entity_record_id(&id))
        {
            continue;
        }
        let rid = crate::notes::entity_record_id(&id);
        if let (Some(previous), Some(remote)) =
            (baseline.get(&id), engine.get_local(Group::Notes, rid).await)
        {
            if previous != &remote {
                let mut copy: crate::notes::SyncEntity =
                    serde_json::from_slice(&remote).map_err(json_err)?;
                let copy_id = Uuid::new_v5(&rid.0, &remote).to_string();
                match &mut copy {
                    crate::notes::SyncEntity::Note(n) => {
                        n.id = copy_id;
                        n.title.push_str(" (conflict)");
                    }
                    crate::notes::SyncEntity::Folder(f) => {
                        f.id = copy_id;
                        f.name.push_str(" (conflict)");
                    }
                }
                engine
                    .put_local_if_changed(
                        Group::Notes,
                        copy.record_id(),
                        serde_json::to_vec(&copy).map_err(json_err)?,
                    )
                    .await?;
            }
        }
        baseline.insert(id.clone(), Vec::new());
        if !engine
            .snapshot_group(Group::Notes)
            .await
            .iter()
            .any(|(id, r)| *id == rid && r.tombstone)
        {
            engine.delete_local(Group::Notes, rid).await?;
        }
    }
    for mut entity in entities {
        // Local optimistic versions are not part of synced content.
        match &mut entity {
            crate::notes::SyncEntity::Note(n) => {
                n.version = 0;
                n.updated_at = 0;
            }
            crate::notes::SyncEntity::Folder(f) => {
                f.version = 0;
                f.updated_at = 0;
            }
        }
        let bytes = serde_json::to_vec(&entity).map_err(json_err)?;
        if baseline.get(entity.id()) == Some(&bytes)
            && engine
                .snapshot_group(Group::Notes)
                .await
                .iter()
                .any(|(id, _)| *id == entity.record_id())
        {
            continue;
        }
        // If reconciliation succeeded but local apply failed, a subsequent local
        // edit is based on the old snapshot, not on the pulled engine payload.
        // Preserve that unseen remote variant before advancing its VV locally.
        if let (Some(previous), Some(remote)) = (
            baseline.get(entity.id()),
            engine.get_local(Group::Notes, entity.record_id()).await,
        ) {
            if previous != &remote && bytes != remote {
                let mut copy: crate::notes::SyncEntity =
                    serde_json::from_slice(&remote).map_err(json_err)?;
                let copy_id = Uuid::new_v5(&entity.record_id().0, &remote).to_string();
                match &mut copy {
                    crate::notes::SyncEntity::Note(n) => {
                        n.id = copy_id;
                        n.title.push_str(" (conflict)");
                    }
                    crate::notes::SyncEntity::Folder(f) => {
                        f.id = copy_id;
                        f.name.push_str(" (conflict)");
                    }
                }
                engine
                    .put_local_if_changed(
                        Group::Notes,
                        copy.record_id(),
                        serde_json::to_vec(&copy).map_err(json_err)?,
                    )
                    .await?;
            }
        }
        engine
            .put_local_if_changed(Group::Notes, entity.record_id(), bytes.clone())
            .await?;
        baseline.insert(entity.id().to_string(), bytes);
    }
    if persist {
        store.set_export_baseline(&baseline).map_err(store_err)?;
    }
    Ok(revision)
}
pub async fn import_notes(
    store: &crate::notes::NotesStore,
    engine: &SyncEngine,
    revision: u64,
) -> Result<(), SyncError> {
    let (local, _, _) = store.sync_snapshot().map_err(store_err)?;
    let mut entities = Vec::new();
    let mut deleted = Vec::new();
    for (id, record) in engine.snapshot_group(Group::Notes).await {
        if record.tombstone {
            for entity in &local {
                if entity.record_id() == id {
                    deleted.push(entity.id().to_string());
                }
            }
        } else {
            let entity: crate::notes::SyncEntity =
                serde_json::from_slice(&record.payload).map_err(json_err)?;
            if entity.record_id() != id {
                return Err(store_err("notes record id mismatch"));
            }
            entities.push(entity);
        }
    }
    store
        .apply_sync(entities, deleted, revision)
        .map_err(store_err)?;
    // Publish deterministic graph repair back into the engine, not merely the
    // local projection; subsequent cycles must converge instead of re-failing.
    for mut entity in store.sync_snapshot().map_err(store_err)?.0 {
        match &mut entity {
            crate::notes::SyncEntity::Note(n) => {
                n.version = 0;
                n.updated_at = 0;
            }
            crate::notes::SyncEntity::Folder(f) => {
                f.version = 0;
                f.updated_at = 0;
            }
        }
        engine
            .put_local_if_changed(
                Group::Notes,
                entity.record_id(),
                serde_json::to_vec(&entity).map_err(json_err)?,
            )
            .await?;
    }
    Ok(())
}

async fn put_local_if_changed(
    engine: &SyncEngine,
    group: Group,
    id: RecordId,
    bytes: Vec<u8>,
) -> Result<(), SyncError> {
    engine.put_local_if_changed(group, id, bytes).await
}

async fn export_connections(profiles: &ProfileStore, engine: &SyncEngine) -> Result<(), SyncError> {
    for id in profiles.explicit_deletions().await.map_err(store_err)? {
        let rid = RecordId(id);
        if engine.get_local(Group::Connections, rid).await.is_some() {
            engine.delete_local(Group::Connections, rid).await?;
        }
    }
    let list = profiles.list().await.map_err(store_err)?;
    let deleted = engine
        .snapshot_group(Group::Connections)
        .await
        .into_iter()
        .filter(|(_, record)| record.tombstone)
        .map(|(id, _)| id)
        .collect::<std::collections::HashSet<_>>();
    for profile in list {
        if deleted.contains(&RecordId(profile.id)) {
            continue;
        }
        let bytes = serde_json::to_vec(&profile).map_err(json_err)?;
        put_local_if_changed(engine, Group::Connections, RecordId(profile.id), bytes).await?;
    }
    Ok(())
}

async fn import_connections(profiles: &ProfileStore, engine: &SyncEngine) -> Result<(), SyncError> {
    for (id, record) in engine.snapshot_group(Group::Connections).await {
        if record.tombstone {
            profiles.delete(id.0).await.map_err(store_err)?;
        }
    }
    for id in engine.list_local(Group::Connections).await {
        if let Some(bytes) = engine.get_local(Group::Connections, id).await {
            let profile: Profile = serde_json::from_slice(&bytes).map_err(json_err)?;
            profiles.upsert_synced(profile).await.map_err(store_err)?;
        }
    }
    Ok(())
}

async fn export_appearance(settings: &SettingsStore, engine: &SyncEngine) -> Result<(), SyncError> {
    export_settings_group(settings, engine, Group::Appearance).await
}

async fn import_appearance(settings: &SettingsStore, engine: &SyncEngine) -> Result<(), SyncError> {
    migrate_legacy_bundle(settings, engine, Group::Appearance, bundle_appearance()).await?;
    import_settings_group(settings, engine, Group::Appearance).await
}

async fn export_shortcuts(settings: &SettingsStore, engine: &SyncEngine) -> Result<(), SyncError> {
    export_settings_group(settings, engine, Group::Shortcuts).await
}

async fn import_shortcuts(settings: &SettingsStore, engine: &SyncEngine) -> Result<(), SyncError> {
    migrate_legacy_bundle(settings, engine, Group::Shortcuts, bundle_shortcuts()).await?;
    import_settings_group(settings, engine, Group::Shortcuts).await
}

async fn export_plugin_cfg(
    settings: &SettingsStore,
    plugins: &[PluginInfo],
    engine: &SyncEngine,
) -> Result<(), SyncError> {
    let bundle = PluginCfgBundle {
        schema: SCHEMA,
        plugins: plugins.to_vec(),
    };
    let bytes = serde_json::to_vec(&bundle).map_err(json_err)?;
    put_local_if_changed(engine, Group::PluginCfg, plugin_list_record(), bytes).await?;
    export_settings_group(settings, engine, Group::PluginCfg).await
}

async fn import_plugin_cfg(settings: &SettingsStore, engine: &SyncEngine) -> Result<(), SyncError> {
    // Legacy bundle: `{plugin_cfg, plugin_settings}` or a bare PluginCfgBundle.
    if let Some(bytes) = engine.get_local(Group::PluginCfg, bundle_plugincfg()).await {
        if let Ok(v) = serde_json::from_slice::<Value>(&bytes) {
            if let Some(extra) = v.get("plugin_settings") {
                if let Ok(bundle) = serde_json::from_value::<SettingsBundle>(extra.clone()) {
                    apply_settings_entries(settings, bundle.entries)?;
                }
            }
        }
        engine
            .delete_local(Group::PluginCfg, bundle_plugincfg())
            .await?;
    }
    // Plugin binaries are not synced; the list record is metadata for other devices.
    import_settings_group(settings, engine, Group::PluginCfg).await
}

/// Export every syncable settings key belonging to `group` as its own record,
/// tombstoning records for keys that no longer exist locally.
async fn export_settings_group(
    settings: &SettingsStore,
    engine: &SyncEngine,
    group: Group,
) -> Result<(), SyncError> {
    let entries = syncable_settings(settings)?
        .into_iter()
        .filter(|e| settings_group_for_key(&e.key) == Some(group))
        .collect::<Vec<_>>();
    let mut live_ids = std::collections::HashSet::new();
    for e in entries {
        let rid = settings_record_id(&e.key);
        live_ids.insert(rid);
        let rec = SettingsKeyRecord {
            schema: SCHEMA,
            key: e.key,
            value: e.value,
        };
        let bytes = serde_json::to_vec(&rec).map_err(json_err)?;
        put_local_if_changed(engine, group, rid, bytes).await?;
    }
    // Keys deleted locally → tombstone their records so the deletion propagates.
    for rid in engine.list_local(group).await {
        if live_ids.contains(&rid) || rid == plugin_list_record() {
            continue;
        }
        let Some(bytes) = engine.get_local(group, rid).await else {
            continue;
        };
        if serde_json::from_slice::<SettingsKeyRecord>(&bytes).is_ok() {
            engine.delete_local(group, rid).await?;
        }
    }
    Ok(())
}

async fn import_settings_group(
    settings: &SettingsStore,
    engine: &SyncEngine,
    group: Group,
) -> Result<(), SyncError> {
    let mut entries = Vec::new();
    for rid in engine.list_local(group).await {
        let Some(bytes) = engine.get_local(group, rid).await else {
            continue;
        };
        let Ok(rec) = serde_json::from_slice::<SettingsKeyRecord>(&bytes) else {
            continue; // plugin list record or foreign payload
        };
        if settings_record_id(&rec.key) != rid {
            continue; // forged / mismatched record
        }
        entries.push(SettingEntry {
            key: rec.key,
            value: rec.value,
        });
    }
    apply_settings_entries(settings, entries)
}

/// One-shot: a legacy whole-group bundle is applied locally, then tombstoned so
/// peers stop seeing it. The next export re-emits the content as per-key records.
async fn migrate_legacy_bundle(
    settings: &SettingsStore,
    engine: &SyncEngine,
    group: Group,
    legacy: RecordId,
) -> Result<(), SyncError> {
    let Some(bytes) = engine.get_local(group, legacy).await else {
        return Ok(());
    };
    if let Ok(bundle) = serde_json::from_slice::<SettingsBundle>(&bytes) {
        apply_settings_entries(settings, bundle.entries)?;
    }
    engine.delete_local(group, legacy).await
}

async fn export_credentials(
    vault: Option<&VaultStore>,
    engine: &SyncEngine,
    notes: &mut BridgeNotes,
) -> Result<(), SyncError> {
    let Some(store) = vault else {
        notes.credentials_skipped_uninitialized = true;
        return Ok(());
    };
    if !store.is_initialized().map_err(vault_store_err)? {
        notes.credentials_skipped_uninitialized = true;
        return Ok(());
    }
    if !store.is_unlocked().await {
        notes.credentials_skipped_locked = true;
        return Ok(());
    }
    for id in store.explicit_deletions().map_err(vault_store_err)? {
        let rid = credential_record_id(&id);
        if engine.get_local(Group::Credentials, rid).await.is_some() {
            engine.delete_local(Group::Credentials, rid).await?;
        }
    }
    let metas = store.list().await.map_err(vault_store_err)?;
    let deleted = engine
        .snapshot_group(Group::Credentials)
        .await
        .into_iter()
        .filter(|(_, record)| record.tombstone)
        .map(|(id, _)| id)
        .collect::<std::collections::HashSet<_>>();
    for meta in metas {
        if deleted.contains(&credential_record_id(&meta.id)) {
            continue;
        }
        let entry = store.get(&meta.id).await.map_err(vault_store_err)?;
        let bytes = serde_json::to_vec(&entry).map_err(json_err)?;
        let rid = credential_record_id(&entry.id);
        put_local_if_changed(engine, Group::Credentials, rid, bytes).await?;
    }
    Ok(())
}

async fn import_credentials(
    vault: Option<&VaultStore>,
    engine: &SyncEngine,
) -> Result<(), SyncError> {
    let Some(store) = vault else {
        return Ok(());
    };
    if !store.is_unlocked().await {
        return Ok(());
    }
    for meta in store.list().await.map_err(vault_store_err)? {
        if engine
            .snapshot_group(Group::Credentials)
            .await
            .iter()
            .any(|(id, record)| *id == credential_record_id(&meta.id) && record.tombstone)
        {
            store.remove(&meta.id).await.map_err(vault_store_err)?;
        }
    }
    for id in engine.list_local(Group::Credentials).await {
        if let Some(bytes) = engine.get_local(Group::Credentials, id).await {
            let entry: VaultEntry = serde_json::from_slice(&bytes).map_err(json_err)?;
            let _ = store.put(entry).await.map_err(vault_store_err)?;
        }
    }
    Ok(())
}

/// Every settings entry that may be synced, with secrets and device-local
/// fields removed.
fn syncable_settings(settings: &SettingsStore) -> Result<Vec<SettingEntry>, SyncError> {
    let all = settings.all().map_err(store_err)?;
    Ok(all
        .into_iter()
        .filter(|e| !is_local_only_key(&e.key))
        .map(|mut e| {
            if e.key == "ai" {
                e.value = redact_ai_value(e.value);
            } else if e.key == "window" {
                e.value = strip_local_window_fields(e.value);
            }
            e
        })
        .collect())
}

fn redact_ai_value(mut v: Value) -> Value {
    if let Some(obj) = v.as_object_mut() {
        obj.remove("apiKey");
    }
    v
}

fn strip_local_window_fields(mut v: Value) -> Value {
    if let Some(obj) = v.as_object_mut() {
        for f in LOCAL_ONLY_WINDOW_FIELDS {
            obj.remove(*f);
        }
    }
    v
}

/// Apply incoming settings, skipping local-only keys and preserving local-only
/// `window` sub-fields and the local `ai.apiKey`.
fn apply_settings_entries(
    settings: &SettingsStore,
    entries: Vec<SettingEntry>,
) -> Result<(), SyncError> {
    for e in entries {
        if is_local_only_key(&e.key) {
            continue;
        }
        let mut value = e.value;
        if e.key == "window" || e.key == "ai" {
            if let (Some(incoming), Ok(Some(Value::Object(local)))) =
                (value.as_object_mut(), settings.get(&e.key))
            {
                let keep: &[&str] = if e.key == "window" {
                    LOCAL_ONLY_WINDOW_FIELDS
                } else {
                    &["apiKey"]
                };
                for f in keep {
                    if let Some(v) = local.get(*f) {
                        incoming.insert((*f).to_string(), v.clone());
                    }
                }
            }
        }
        if settings.get(&e.key).map_err(store_err)?.as_ref() == Some(&value) {
            continue;
        }
        settings.set(&e.key, &value).map_err(store_err)?;
    }
    Ok(())
}

fn credential_record_id(id: &str) -> RecordId {
    RecordId(
        Uuid::parse_str(id).unwrap_or_else(|_| Uuid::new_v5(&Uuid::NAMESPACE_OID, id.as_bytes())),
    )
}

fn store_err(e: impl std::fmt::Display) -> SyncError {
    SyncError::Transport(format!("store: {e}"))
}

fn vault_store_err(e: impl std::fmt::Display) -> SyncError {
    SyncError::Transport(format!("vault: {e}"))
}

fn json_err(e: serde_json::Error) -> SyncError {
    SyncError::Transport(format!("json: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::crypto::KdfParams;
    use crate::sync::SyncBackend;
    use async_trait::async_trait;
    use serde_json::json;
    use std::sync::Arc;

    struct NullBackend;

    #[async_trait]
    impl SyncBackend for NullBackend {
        async fn list(&self, _g: Group) -> Result<Vec<RecordId>, SyncError> {
            Ok(vec![])
        }
        async fn get(&self, _g: Group, _id: RecordId) -> Result<Vec<u8>, SyncError> {
            Err(SyncError::Transport("null".into()))
        }
        async fn put(&self, _g: Group, _id: RecordId, _b: &[u8]) -> Result<(), SyncError> {
            Ok(())
        }
        async fn delete(&self, _g: Group, _id: RecordId) -> Result<(), SyncError> {
            Ok(())
        }
    }

    fn engine() -> SyncEngine {
        SyncEngine::new(
            Uuid::new_v4(),
            b"pw".to_vec(),
            KdfParams::test_cheap(),
            Arc::new(NullBackend),
        )
    }

    fn settings() -> SettingsStore {
        let mut p = std::env::temp_dir();
        p.push(format!("aerotab-bridge-test-{}", Uuid::new_v4()));
        SettingsStore::open(p).unwrap()
    }

    fn profile(id: Uuid) -> Profile {
        serde_json::from_value(json!({"id":id,"name":"test","kind":"ssh","ssh":{"host":"host","port":22,"user":"user","auth":"Agent"}})).unwrap()
    }

    #[derive(Default)]
    struct NotesMemoryBackend(
        std::sync::Mutex<std::collections::HashMap<(Group, RecordId), Vec<u8>>>,
    );
    #[async_trait]
    impl SyncBackend for NotesMemoryBackend {
        async fn list(&self, g: Group) -> Result<Vec<RecordId>, SyncError> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .keys()
                .filter(|(group, _)| *group == g)
                .map(|(_, id)| *id)
                .collect())
        }
        async fn get(&self, g: Group, id: RecordId) -> Result<Vec<u8>, SyncError> {
            self.0
                .lock()
                .unwrap()
                .get(&(g, id))
                .cloned()
                .ok_or_else(|| store_err("missing"))
        }
        async fn put(&self, g: Group, id: RecordId, b: &[u8]) -> Result<(), SyncError> {
            self.0.lock().unwrap().insert((g, id), b.to_vec());
            Ok(())
        }
        async fn delete(&self, g: Group, id: RecordId) -> Result<(), SyncError> {
            self.0.lock().unwrap().remove(&(g, id));
            Ok(())
        }
    }
    async fn notes_cycle(
        store: &crate::notes::NotesStore,
        e: &SyncEngine,
    ) -> Result<(), SyncError> {
        let rev = export_notes(store, e).await?;
        e.sync_group(Group::Notes).await?;
        import_notes(store, e, rev).await
    }
    fn put_note(store: &crate::notes::NotesStore, body: &str) {
        let current = crate::notes::dispatch(store, "notes.list", json!({}), &[]).unwrap();
        let version = current["notes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["id"] == "n")
            .map_or(0, |n| n["version"].as_u64().unwrap());
        crate::notes::dispatch(store,"notes.put",json!({"note":{"id":"n","parent_id":null,"title":"N","markdown":body,"tags":["tag"],"profile_ids":["missing"]},"expectedVersion":version}),&["missing".into()]).unwrap();
    }
    #[tokio::test]
    async fn two_notes_stores_converge_edit_delete_and_stable_conflict_ids() {
        let backend = Arc::new(NotesMemoryBackend::default());
        let a = SyncEngine::new(
            Uuid::new_v4(),
            b"pw".to_vec(),
            KdfParams::test_cheap(),
            backend.clone(),
        );
        let b = SyncEngine::new(
            Uuid::new_v4(),
            b"pw".to_vec(),
            KdfParams::test_cheap(),
            backend,
        );
        let sa =
            crate::notes::NotesStore::open(std::env::temp_dir().join(Uuid::new_v4().to_string()))
                .unwrap();
        let sb =
            crate::notes::NotesStore::open(std::env::temp_dir().join(Uuid::new_v4().to_string()))
                .unwrap();
        put_note(&sa, "base");
        notes_cycle(&sa, &a).await.unwrap();
        notes_cycle(&sb, &b).await.unwrap();
        put_note(&sa, "left");
        put_note(&sb, "right");
        for _ in 0..3 {
            notes_cycle(&sa, &a).await.unwrap();
            notes_cycle(&sb, &b).await.unwrap();
        }
        let content = |s: &crate::notes::NotesStore| {
            s.sync_snapshot()
                .unwrap()
                .0
                .into_iter()
                .map(|entity| {
                    let crate::notes::SyncEntity::Note(n) = entity else {
                        panic!()
                    };
                    (n.id, n.markdown, n.profile_ids)
                })
                .collect::<std::collections::BTreeSet<_>>()
        };
        assert_eq!(content(&sa), content(&sb));
        let stable = content(&sa);
        assert!(stable.iter().any(|(_, body, _)| body == "left"));
        assert!(stable.iter().any(|(_, body, _)| body == "right"));
        for _ in 0..3 {
            notes_cycle(&sa, &a).await.unwrap();
            notes_cycle(&sb, &b).await.unwrap();
        }
        assert_eq!(stable, content(&sa));
        let current = crate::notes::dispatch(&sa, "notes.list", json!({}), &[]).unwrap();
        let version = current["notes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["id"] == "n")
            .unwrap()["version"]
            .clone();
        crate::notes::dispatch(
            &sa,
            "notes.delete",
            json!({"id":"n","expectedVersion":version}),
            &[],
        )
        .unwrap();
        put_note(&sb, "edit against delete");
        for _ in 0..4 {
            notes_cycle(&sa, &a).await.unwrap();
            notes_cycle(&sb, &b).await.unwrap();
        }
        assert_eq!(content(&sa), content(&sb));
        assert!(!content(&sa).iter().any(|(id, _, _)| id == "n"));
        assert!(content(&sa)
            .iter()
            .any(|(_, body, _)| body == "edit against delete"));
    }
    #[tokio::test]
    async fn two_notes_stores_repair_concurrent_folder_ring_and_converge() {
        let backend = Arc::new(NotesMemoryBackend::default());
        let a = SyncEngine::new(
            Uuid::new_v4(),
            b"pw".to_vec(),
            KdfParams::test_cheap(),
            backend.clone(),
        );
        let b = SyncEngine::new(
            Uuid::new_v4(),
            b"pw".to_vec(),
            KdfParams::test_cheap(),
            backend,
        );
        let sa =
            crate::notes::NotesStore::open(std::env::temp_dir().join(Uuid::new_v4().to_string()))
                .unwrap();
        let sb =
            crate::notes::NotesStore::open(std::env::temp_dir().join(Uuid::new_v4().to_string()))
                .unwrap();
        for id in ["a", "b"] {
            crate::notes::dispatch(
                &sa,
                "notes.putFolder",
                json!({"folder":{"id":id,"parent_id":null,"name":id},"expectedVersion":0}),
                &[],
            )
            .unwrap();
        }
        notes_cycle(&sa, &a).await.unwrap();
        notes_cycle(&sb, &b).await.unwrap();
        for (store, id, parent) in [(&sa, "a", "b"), (&sb, "b", "a")] {
            let snapshot = crate::notes::dispatch(store, "notes.list", json!({}), &[]).unwrap();
            let f = snapshot["folders"]
                .as_array()
                .unwrap()
                .iter()
                .find(|f| f["id"] == id)
                .unwrap();
            let mut changed = f.clone();
            changed["parent_id"] = json!(parent);
            crate::notes::dispatch(
                store,
                "notes.putFolder",
                json!({"folder":changed,"expectedVersion":f["version"]}),
                &[],
            )
            .unwrap();
        }
        for _ in 0..5 {
            notes_cycle(&sa, &a).await.unwrap();
            notes_cycle(&sb, &b).await.unwrap();
        }
        let graph = |s: &crate::notes::NotesStore| {
            s.sync_snapshot()
                .unwrap()
                .0
                .into_iter()
                .map(|e| {
                    let crate::notes::SyncEntity::Folder(f) = e else {
                        panic!()
                    };
                    (f.id, f.parent_id, f.recovered_parent_id)
                })
                .collect::<std::collections::BTreeSet<_>>()
        };
        assert_eq!(graph(&sa), graph(&sb));
        assert!(graph(&sa).contains(&("a".into(), None, Some("b".into()))));
        let before = a
            .snapshot_group(Group::Notes)
            .await
            .into_iter()
            .map(|(id, r)| (id.0, r.vv))
            .collect::<std::collections::BTreeMap<_, _>>();
        notes_cycle(&sa, &a).await.unwrap();
        let after = a
            .snapshot_group(Group::Notes)
            .await
            .into_iter()
            .map(|(id, r)| (id.0, r.vv))
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(before, after);
        for s in [&sa, &sb] {
            let (entities, _, _) = s.sync_snapshot().unwrap();
            let roots = entities
                .iter()
                .filter(
                    |e| matches!(e,crate::notes::SyncEntity::Folder(f) if f.parent_id.is_none()),
                )
                .count();
            assert_eq!(roots, 1, "invalid ring must never enter local store");
        }
    }

    #[tokio::test]
    async fn failed_notes_import_must_not_reexport_old_snapshot() {
        let store =
            crate::notes::NotesStore::open(std::env::temp_dir().join(Uuid::new_v4().to_string()))
                .unwrap();
        crate::notes::dispatch(&store,"notes.put",json!({"note":{"id":"n","parent_id":null,"title":"N","markdown":"old"},"expectedVersion":0}),&[]).unwrap();
        let e = engine();
        let revision = export_notes(&store, &e).await.unwrap();
        let baseline = store.export_baseline().unwrap();
        assert!(!baseline.is_empty());
        let id = crate::notes::entity_record_id("n");
        let mut remote: Value =
            serde_json::from_slice(&e.get_local(Group::Notes, id).await.unwrap()).unwrap();
        remote["value"]["markdown"] = json!("remote");
        e.put_local(Group::Notes, id, serde_json::to_vec(&remote).unwrap())
            .await
            .unwrap();
        // A failed revision guard must not turn old local content into a new write.
        assert!(import_notes(&store, &e, revision + 1).await.is_err());
        let before = e.snapshot_group(Group::Notes).await;
        export_notes(&store, &e).await.unwrap();
        assert_eq!(
            e.get_local(Group::Notes, id).await.unwrap(),
            serde_json::to_vec(&remote).unwrap()
        );
        assert_eq!(e.snapshot_group(Group::Notes).await[0].1.vv, before[0].1.vv);
        let snapshot = crate::notes::dispatch(&store, "notes.list", json!({}), &[]).unwrap();
        let mut local = snapshot["notes"][0].clone();
        local["markdown"] = json!("new draft after failure");
        crate::notes::dispatch(
            &store,
            "notes.put",
            json!({"note":local,"expectedVersion":snapshot["notes"][0]["version"]}),
            &[],
        )
        .unwrap();
        export_notes(&store, &e).await.unwrap();
        let bodies: Vec<_> = e
            .snapshot_group(Group::Notes)
            .await
            .into_iter()
            .filter(|(_, r)| !r.tombstone)
            .map(|(_, r)| {
                serde_json::from_slice::<Value>(&r.payload).unwrap()["value"]["markdown"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect();
        assert!(
            bodies.contains(&"remote".into()) && bodies.contains(&"new draft after failure".into())
        );
    }

    #[tokio::test]
    async fn notes_bridge_roundtrip_tombstones_and_no_appearance_leak() {
        let src =
            crate::notes::NotesStore::open(std::env::temp_dir().join(Uuid::new_v4().to_string()))
                .unwrap();
        let dst =
            crate::notes::NotesStore::open(std::env::temp_dir().join(Uuid::new_v4().to_string()))
                .unwrap();
        let note = serde_json::json!({"id":"n","parent_id":"f","title":"N","markdown":"# body","tags":["tag"],"profile_ids":["missing"]});
        crate::notes::dispatch(
            &src,
            "notes.putFolder",
            json!({"folder":{"id":"f","parent_id":null,"name":"F"},"expectedVersion":0}),
            &[],
        )
        .unwrap();
        let saved = crate::notes::dispatch(
            &src,
            "notes.put",
            json!({"note":note,"expectedVersion":0}),
            &["missing".into()],
        )
        .unwrap();
        let e = engine();
        export_notes(&src, &e).await.unwrap();
        export_notes(&dst, &e).await.unwrap(); // first sync empty device cannot erase peers
        import_notes(&dst, &e, 0).await.unwrap();
        let pulled = crate::notes::dispatch(&dst, "notes.list", json!({}), &[]).unwrap();
        assert_eq!(pulled["notes"][0]["markdown"], "# body");
        assert_eq!(pulled["notes"][0]["tags"], json!(["tag"]));
        assert_eq!(pulled["notes"][0]["profile_ids"], json!(["missing"]));
        crate::notes::dispatch(
            &src,
            "notes.delete",
            json!({"id":"n","expectedVersion":saved["version"]}),
            &[],
        )
        .unwrap();
        export_notes(&src, &e).await.unwrap();
        let revision = dst.sync_snapshot().unwrap().2;
        import_notes(&dst, &e, revision).await.unwrap();
        assert_eq!(
            crate::notes::dispatch(&dst, "notes.list", json!({}), &[]).unwrap()["notes"],
            json!([])
        );
        assert_eq!(settings_group_for_key("notes"), None);
        assert_eq!(settings_group_for_key("notes.drafts"), None);
        assert_eq!(Group::from_slug("notes"), Some(Group::Notes));
    }

    #[tokio::test]
    async fn profile_explicit_deletion_propagates_without_first_sync_data_loss() {
        let src =
            ProfileStore::open(std::env::temp_dir().join(Uuid::new_v4().to_string())).unwrap();
        let dst =
            ProfileStore::open(std::env::temp_dir().join(Uuid::new_v4().to_string())).unwrap();
        let e = engine();
        let p = profile(Uuid::new_v4());
        src.upsert(p.clone()).await.unwrap();
        export_connections(&src, &e).await.unwrap();
        export_connections(&dst, &e).await.unwrap();
        assert!(e
            .get_local(Group::Connections, RecordId(p.id))
            .await
            .is_some());
        import_connections(&dst, &e).await.unwrap();
        src.delete(p.id).await.unwrap();
        export_connections(&src, &e).await.unwrap();
        assert!(
            e.get_local(Group::Connections, RecordId(p.id))
                .await
                .is_none(),
            "explicit deletion must tombstone sync record"
        );
        import_connections(&dst, &e).await.unwrap();
        assert!(dst.get(p.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn vault_explicit_deletion_propagates_without_first_sync_data_loss() {
        let src = VaultStore::open_with_params(
            std::env::temp_dir().join(Uuid::new_v4().to_string()),
            KdfParams::test_cheap(),
        )
        .unwrap();
        let dst = VaultStore::open_with_params(
            std::env::temp_dir().join(Uuid::new_v4().to_string()),
            KdfParams::test_cheap(),
        )
        .unwrap();
        src.initialize(b"pw").await.unwrap();
        dst.initialize(b"pw").await.unwrap();
        let entry = src
            .put(VaultEntry {
                id: "legacy-key-id".into(),
                label: "key".into(),
                kind: crate::vault::EntryKind::Password,
                username: None,
                secret: "secret".into(),
            })
            .await
            .unwrap();
        let e = engine();
        let mut notes = BridgeNotes::default();
        export_credentials(Some(&src), &e, &mut notes)
            .await
            .unwrap();
        export_credentials(Some(&dst), &e, &mut notes)
            .await
            .unwrap();
        assert!(e
            .get_local(Group::Credentials, credential_record_id(&entry.id))
            .await
            .is_some());
        import_credentials(Some(&dst), &e).await.unwrap();
        src.remove(&entry.id).await.unwrap();
        export_credentials(Some(&src), &e, &mut notes)
            .await
            .unwrap();
        assert!(
            e.get_local(Group::Credentials, credential_record_id(&entry.id))
                .await
                .is_none(),
            "explicit vault deletion must tombstone sync record"
        );
        import_credentials(Some(&dst), &e).await.unwrap();
        assert!(matches!(
            dst.get(&entry.id).await,
            Err(VaultError::NotFound(_))
        ));
    }

    #[test]
    fn local_only_keys_are_recognised() {
        assert!(is_local_only_key("sync"));
        assert!(is_local_only_key("mcp"));
        assert!(is_local_only_key("mcp.endpoint"));
        assert_eq!(settings_group_for_key("mcp"), None);
        assert!(is_local_only_key("syncHistory"));
        assert!(is_local_only_key("openTabs"));
        assert!(is_local_only_key("diagnostics"));
        assert!(is_local_only_key("diagnostics.log"));
        assert!(!is_local_only_key("theme"));
        assert!(!is_local_only_key("hotkeys"));
        assert_eq!(settings_group_for_key("hotkeys"), Some(Group::Shortcuts));
        assert_eq!(settings_group_for_key("plugin.foo"), Some(Group::PluginCfg));
        assert_eq!(settings_group_for_key("theme"), Some(Group::Appearance));
        assert_eq!(settings_group_for_key("sync"), None);
    }

    #[tokio::test]
    async fn export_is_per_key_and_skips_local_only() {
        let s = settings();
        s.set("theme", &json!("dark")).unwrap();
        s.set("sync", &json!({"backend": "git"})).unwrap();
        s.set("openTabs", &json!([1, 2])).unwrap();
        s.set("window", &json!({"opacity": 90, "sidebarVisible": false}))
            .unwrap();
        s.set("hotkeys", &json!({"a": "b"})).unwrap();
        let e = engine();
        export_appearance(&s, &e).await.unwrap();
        export_shortcuts(&s, &e).await.unwrap();

        let mut ids = e.list_local(Group::Appearance).await;
        ids.sort_by_key(|r| r.0);
        let mut expect = vec![settings_record_id("theme"), settings_record_id("window")];
        expect.sort_by_key(|r| r.0);
        assert_eq!(ids, expect);
        assert_eq!(
            e.list_local(Group::Shortcuts).await,
            vec![settings_record_id("hotkeys")]
        );

        let win = e
            .get_local(Group::Appearance, settings_record_id("window"))
            .await
            .unwrap();
        let rec: SettingsKeyRecord = serde_json::from_slice(&win).unwrap();
        assert_eq!(rec.value, json!({"opacity": 90}));

        // Re-export without changes must not bump anything.
        export_appearance(&s, &e).await.unwrap();
        assert_eq!(e.list_local(Group::Appearance).await.len(), 2);
    }

    #[tokio::test]
    async fn import_preserves_local_window_fields_and_removes_deleted_keys() {
        let src = settings();
        src.set("theme", &json!("dark")).unwrap();
        src.set("window", &json!({"opacity": 70})).unwrap();
        let e = engine();
        export_appearance(&src, &e).await.unwrap();

        let dst = settings();
        dst.set("window", &json!({"opacity": 100, "sidebarVisible": true}))
            .unwrap();
        import_appearance(&dst, &e).await.unwrap();
        assert_eq!(dst.get("theme").unwrap().unwrap(), json!("dark"));
        assert_eq!(
            dst.get("window").unwrap().unwrap(),
            json!({"opacity": 70, "sidebarVisible": true})
        );

        // Deleting a key locally tombstones its record on the next export.
        src.remove("theme").unwrap();
        export_appearance(&src, &e).await.unwrap();
        assert_eq!(
            e.list_local(Group::Appearance).await,
            vec![settings_record_id("window")]
        );
    }

    #[tokio::test]
    async fn legacy_bundle_is_applied_once_then_tombstoned() {
        let e = engine();
        let bundle = SettingsBundle {
            schema: SCHEMA,
            entries: vec![
                SettingEntry {
                    key: "theme".into(),
                    value: json!("legacy"),
                },
                SettingEntry {
                    key: "sync".into(),
                    value: json!({"secret": 1}),
                },
            ],
        };
        e.put_local(
            Group::Appearance,
            bundle_appearance(),
            serde_json::to_vec(&bundle).unwrap(),
        )
        .await
        .unwrap();
        let s = settings();
        import_appearance(&s, &e).await.unwrap();
        assert_eq!(s.get("theme").unwrap().unwrap(), json!("legacy"));
        assert!(s.get("sync").unwrap().is_none());
        assert!(e
            .get_local(Group::Appearance, bundle_appearance())
            .await
            .is_none());
        // Next export emits the per-key record.
        export_appearance(&s, &e).await.unwrap();
        assert_eq!(
            e.list_local(Group::Appearance).await,
            vec![settings_record_id("theme")]
        );
    }
}
