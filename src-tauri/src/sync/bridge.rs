//! Export local stores into the sync engine before `sync.now`, and apply pulled
//! records back into Profile / Settings / Vault after reconciliation.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::plugins::wasm_host::PluginInfo;
use crate::profile::{Profile, ProfileStore};
use crate::secret;
use crate::settings::{SettingEntry, SettingsStore};
use crate::sync::{Group, RecordId, SyncEngine, SyncError};
use crate::vault::{VaultEntry, VaultError, VaultStore};

const SCHEMA: u32 = 1;

fn bundle_appearance() -> RecordId {
    RecordId(Uuid::parse_str("a0000001-0001-4000-8000-000000000001").unwrap())
}
fn bundle_shortcuts() -> RecordId {
    RecordId(Uuid::parse_str("a0000001-0001-4000-8000-000000000002").unwrap())
}
fn bundle_plugincfg() -> RecordId {
    RecordId(Uuid::parse_str("a0000001-0001-4000-8000-000000000003").unwrap())
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct BridgeNotes {
    pub credentials_skipped_locked: bool,
    pub credentials_skipped_uninitialized: bool,
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
            Group::Connections => import_connections(profiles, engine).await?,
            Group::Appearance => import_appearance(settings, engine).await?,
            Group::Shortcuts => import_shortcuts(settings, engine).await?,
            Group::PluginCfg => import_plugin_cfg(engine).await?,
            Group::Credentials => import_credentials(vault, engine).await?,
        }
    }
    Ok(())
}

async fn export_connections(profiles: &ProfileStore, engine: &SyncEngine) -> Result<(), SyncError> {
    let list = profiles.list().await.map_err(store_err)?;
    for profile in list {
        let bytes = serde_json::to_vec(&profile).map_err(json_err)?;
        engine
            .put_local(Group::Connections, RecordId(profile.id), bytes)
            .await?;
    }
    Ok(())
}

async fn import_connections(profiles: &ProfileStore, engine: &SyncEngine) -> Result<(), SyncError> {
    for id in engine.list_local(Group::Connections).await {
        if let Some(bytes) = engine.get_local(Group::Connections, id).await {
            let profile: Profile = serde_json::from_slice(&bytes).map_err(json_err)?;
            profiles.upsert(profile).await.map_err(store_err)?;
        }
    }
    Ok(())
}

async fn export_appearance(settings: &SettingsStore, engine: &SyncEngine) -> Result<(), SyncError> {
    let entries = redacted_settings_all(settings)?;
    let bundle = SettingsBundle {
        schema: SCHEMA,
        entries,
    };
    let bytes = serde_json::to_vec(&bundle).map_err(json_err)?;
    engine
        .put_local(Group::Appearance, bundle_appearance(), bytes)
        .await?;
    Ok(())
}

async fn import_appearance(settings: &SettingsStore, engine: &SyncEngine) -> Result<(), SyncError> {
    if let Some(bytes) = engine
        .get_local(Group::Appearance, bundle_appearance())
        .await
    {
        apply_settings_bundle(settings, &bytes)?;
    }
    Ok(())
}

async fn export_shortcuts(settings: &SettingsStore, engine: &SyncEngine) -> Result<(), SyncError> {
    let entries = match settings.get("hotkeys").map_err(store_err)? {
        Some(value) => vec![SettingEntry {
            key: "hotkeys".into(),
            value,
        }],
        None => vec![],
    };
    let bundle = SettingsBundle {
        schema: SCHEMA,
        entries,
    };
    let bytes = serde_json::to_vec(&bundle).map_err(json_err)?;
    engine
        .put_local(Group::Shortcuts, bundle_shortcuts(), bytes)
        .await?;
    Ok(())
}

async fn import_shortcuts(settings: &SettingsStore, engine: &SyncEngine) -> Result<(), SyncError> {
    if let Some(bytes) = engine
        .get_local(Group::Shortcuts, bundle_shortcuts())
        .await
    {
        apply_settings_bundle(settings, &bytes)?;
    }
    Ok(())
}

async fn export_plugin_cfg(
    settings: &SettingsStore,
    plugins: &[PluginInfo],
    engine: &SyncEngine,
) -> Result<(), SyncError> {
    let mut plugin_settings = Vec::new();
    if let Ok(all) = settings.all() {
        for e in all {
            if e.key.starts_with("plugin.") {
                plugin_settings.push(e);
            }
        }
    }
    let bundle = PluginCfgBundle {
        schema: SCHEMA,
        plugins: plugins.to_vec(),
    };
    let mut bytes = serde_json::to_vec(&bundle).map_err(json_err)?;
    if !plugin_settings.is_empty() {
        let extra = SettingsBundle {
            schema: SCHEMA,
            entries: plugin_settings,
        };
        bytes = serde_json::to_vec(&json!({
            "plugin_cfg": bundle,
            "plugin_settings": extra,
        }))
        .map_err(json_err)?;
    }
    engine
        .put_local(Group::PluginCfg, bundle_plugincfg(), bytes)
        .await?;
    Ok(())
}

async fn import_plugin_cfg(engine: &SyncEngine) -> Result<(), SyncError> {
    let _ = engine
        .get_local(Group::PluginCfg, bundle_plugincfg())
        .await;
    // Plugin binaries are not synced; only metadata lands in the engine for other devices.
    Ok(())
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
    let metas = store.list().await.map_err(vault_store_err)?;
    for meta in metas {
        let entry = store.get(&meta.id).await.map_err(vault_store_err)?;
        let bytes = serde_json::to_vec(&entry).map_err(json_err)?;
        let rid = credential_record_id(&entry.id);
        engine.put_local(Group::Credentials, rid, bytes).await?;
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
    for id in engine.list_local(Group::Credentials).await {
        if let Some(bytes) = engine.get_local(Group::Credentials, id).await {
            let entry: VaultEntry = serde_json::from_slice(&bytes).map_err(json_err)?;
            let _ = store.put(entry).await.map_err(vault_store_err)?;
        }
    }
    Ok(())
}

fn redacted_settings_all(settings: &SettingsStore) -> Result<Vec<SettingEntry>, SyncError> {
    let all = settings.all().map_err(store_err)?;
    Ok(all
        .into_iter()
        .map(|mut e| {
            if e.key == "sync" {
                e.value = redact_sync_value(e.value);
            }
            if e.key == "ai" {
                e.value = redact_ai_value(e.value);
            }
            e
        })
        .collect())
}

fn redact_sync_value(mut v: Value) -> Value {
    if let Some(obj) = v.as_object_mut() {
        obj.remove("gitRemotePassword");
        obj.remove("webdavPassword");
    }
    v
}

fn redact_ai_value(mut v: Value) -> Value {
    if let Some(obj) = v.as_object_mut() {
        obj.remove("apiKey");
    }
    v
}

fn apply_settings_bundle(settings: &SettingsStore, bytes: &[u8]) -> Result<(), SyncError> {
    let bundle: SettingsBundle = serde_json::from_slice(bytes).map_err(json_err)?;
    for e in bundle.entries {
        settings.set(&e.key, &e.value).map_err(store_err)?;
    }
    Ok(())
}

fn credential_record_id(id: &str) -> RecordId {
    RecordId(
        Uuid::parse_str(id)
            .unwrap_or_else(|_| Uuid::new_v5(&Uuid::NAMESPACE_OID, id.as_bytes())),
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
