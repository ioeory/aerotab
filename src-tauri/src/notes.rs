//! Ordinary notes only. This store is intentionally outside Appearance sync and Vault.
//! Integration: `NotesStore::open(data_dir.join("notes"))`; call `dispatch` for notes.*.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone)]
pub struct NotesStore {
    tree: sled::Tree,
    lock: Arc<Mutex<()>>,
    editors: Arc<Mutex<BTreeMap<String, bool>>>,
}
#[derive(Clone, Default, Serialize, Deserialize)]
struct Snapshot {
    notes: BTreeMap<String, Note>,
    folders: BTreeMap<String, Folder>,
    revision: u64,
    #[serde(default)]
    deleted: BTreeMap<String, String>,
}

/// Stable, per-entity plaintext inside the encrypted sync envelope.
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value")]
pub enum SyncEntity {
    Note(Note),
    Folder(Folder),
}
impl SyncEntity {
    pub fn id(&self) -> &str {
        match self {
            Self::Note(n) => &n.id,
            Self::Folder(f) => &f.id,
        }
    }
    pub fn record_id(&self) -> crate::sync::RecordId {
        entity_record_id(self.id())
    }
    fn canonical(&self) -> Result<Vec<u8>, String> {
        let mut entity = self.clone();
        match &mut entity {
            Self::Note(n) => {
                n.version = 0;
                n.updated_at = 0;
            }
            Self::Folder(f) => {
                f.version = 0;
                f.updated_at = 0;
            }
        }
        serde_json::to_vec(&entity).map_err(|e| e.to_string())
    }
}
pub fn entity_record_id(id: &str) -> crate::sync::RecordId {
    crate::sync::RecordId(uuid::Uuid::new_v5(
        &uuid::Uuid::NAMESPACE_URL,
        format!("aerotab:notes:{id}").as_bytes(),
    ))
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Note {
    pub id: String,
    pub parent_id: Option<String>,
    pub title: String,
    pub markdown: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub profile_ids: Vec<String>,
    #[serde(default)]
    pub version: u64,
    #[serde(default)]
    pub updated_at: u64,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Folder {
    pub id: String,
    pub parent_id: Option<String>,
    pub name: String,
    /// Original edge cut during deterministic sync graph recovery.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovered_parent_id: Option<String>,
    #[serde(default)]
    pub version: u64,
    #[serde(default)]
    pub updated_at: u64,
}
impl NotesStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, String> {
        let db = sled::open(path).map_err(|e| e.to_string())?;
        Self::from_db(&db)
    }
    pub fn from_db(db: &sled::Db) -> Result<Self, String> {
        Ok(Self {
            tree: db
                .open_tree("ordinary_notes_v1")
                .map_err(|e| e.to_string())?,
            lock: Arc::new(Mutex::new(())),
            editors: Arc::default(),
        })
    }
    /// Dirty ownership is fail-closed: loss of heartbeats never authorizes apply.
    pub fn edit_state(&self, owner: String, dirty: bool) -> Result<(), String> {
        let mut editors = self
            .editors
            .lock()
            .map_err(|_| "notes editors unavailable")?;
        if dirty {
            editors.insert(owner, true);
        } else {
            editors.remove(&owner);
        }
        Ok(())
    }
    pub fn sync_paused(&self) -> bool {
        self.editors.lock().map_or(true, |e| !e.is_empty())
    }
    pub fn export_baseline(&self) -> Result<BTreeMap<String, Vec<u8>>, String> {
        self.tree
            .get(b"sync_export_baseline")
            .map_err(|e| e.to_string())?
            .map(|b| serde_json::from_slice(&b).map_err(|e| e.to_string()))
            .unwrap_or_else(|| Ok(BTreeMap::new()))
    }
    pub fn set_export_baseline(&self, baseline: &BTreeMap<String, Vec<u8>>) -> Result<(), String> {
        self.tree
            .insert(
                b"sync_export_baseline",
                serde_json::to_vec(baseline).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
        self.tree.flush().map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn sync_snapshot(&self) -> Result<(Vec<SyncEntity>, Vec<String>, u64), String> {
        let _guard = self.lock.lock().map_err(|_| "notes lock unavailable")?;
        let s = self.load()?;
        Ok((
            s.notes
                .into_values()
                .map(SyncEntity::Note)
                .chain(s.folders.into_values().map(SyncEntity::Folder))
                .collect(),
            s.deleted.into_keys().collect(),
            s.revision,
        ))
    }
    /// Apply atomically only if no local save occurred since export. A failed
    /// compare leaves all edits intact and the next cycle exports them first.
    pub fn apply_sync(
        &self,
        entities: Vec<SyncEntity>,
        deleted: Vec<String>,
        expected: u64,
    ) -> Result<(), String> {
        let _guard = self.lock.lock().map_err(|_| "notes lock unavailable")?;
        let _editors = self
            .editors
            .lock()
            .map_err(|_| "notes editors unavailable")?;
        if !_editors.is_empty() {
            return Err("Notes sync paused: unsaved draft retained".into());
        }
        let mut s = self.load()?;
        if s.revision != expected {
            return Err("notes changed during sync; retry sync; local edits retained".into());
        }
        for id in deleted {
            s.notes.remove(&id);
            s.folders.remove(&id);
        }
        let next = s.revision + 1;
        for entity in entities {
            match entity {
                SyncEntity::Note(mut n) => {
                    if let Some(old) = s.notes.get(&n.id) {
                        if SyncEntity::Note(old.clone()).canonical()?
                            == SyncEntity::Note(n.clone()).canonical()?
                        {
                            continue;
                        }
                    }
                    n.version = next;
                    s.deleted.remove(&n.id);
                    s.notes.insert(n.id.clone(), n);
                }
                SyncEntity::Folder(mut f) => {
                    if let Some(old) = s.folders.get(&f.id) {
                        if SyncEntity::Folder(old.clone()).canonical()?
                            == SyncEntity::Folder(f.clone()).canonical()?
                        {
                            continue;
                        }
                    }
                    f.version = next;
                    s.deleted.remove(&f.id);
                    s.folders.insert(f.id.clone(), f);
                }
            }
        }
        // Orphans and cycles recover deterministically; original edges remain metadata.
        let folder_ids: std::collections::HashSet<_> = s.folders.keys().cloned().collect();
        for f in s.folders.values_mut() {
            if f.parent_id
                .as_ref()
                .is_some_and(|p| !folder_ids.contains(p))
            {
                f.recovered_parent_id = f.parent_id.take();
            }
        }
        // Functional parent graph: for each ring cut the lexicographically
        // smallest stable ID. All peers derive the same edge, without clocks.
        let ids: Vec<_> = s.folders.keys().cloned().collect();
        for id in ids {
            let mut path = Vec::<String>::new();
            let mut cursor = Some(id);
            while let Some(id) = cursor {
                if let Some(start) = path.iter().position(|p| p == &id) {
                    let cut = path[start..].iter().min().unwrap().clone();
                    let folder = s.folders.get_mut(&cut).unwrap();
                    folder.recovered_parent_id = folder.parent_id.take();
                    folder.version = next;
                    break;
                }
                path.push(id.clone());
                cursor = s.folders.get(&id).and_then(|f| f.parent_id.clone());
            }
        }
        for n in s.notes.values_mut() {
            if n.parent_id
                .as_ref()
                .is_some_and(|p| !folder_ids.contains(p))
            {
                n.parent_id = None;
            }
        }
        for f in s.folders.values() {
            parent_valid(&s, &f.id, &f.parent_id)?;
        }
        s.revision = next;
        let mut baseline = self.export_baseline()?;
        for n in s.notes.values() {
            let mut n = n.clone();
            n.version = 0;
            n.updated_at = 0;
            baseline.insert(
                n.id.clone(),
                serde_json::to_vec(&SyncEntity::Note(n)).map_err(|e| e.to_string())?,
            );
        }
        for f in s.folders.values() {
            let mut f = f.clone();
            f.version = 0;
            f.updated_at = 0;
            baseline.insert(
                f.id.clone(),
                serde_json::to_vec(&SyncEntity::Folder(f)).map_err(|e| e.to_string())?,
            );
        }
        let mut batch = sled::Batch::default();
        batch.insert(
            b"snapshot",
            serde_json::to_vec(&s).map_err(|e| e.to_string())?,
        );
        batch.insert(
            b"sync_export_baseline",
            serde_json::to_vec(&baseline).map_err(|e| e.to_string())?,
        );
        self.tree.apply_batch(batch).map_err(|e| e.to_string())?;
        self.tree.flush().map_err(|e| e.to_string())?;
        Ok(())
    }
    /// Shared resource deletion must call this before deleting a profile.
    pub fn profile_references(&self, profile_id: &str) -> Result<Vec<String>, String> {
        let _guard = self.lock.lock().map_err(|_| "notes lock unavailable")?;
        Ok(self
            .load()?
            .notes
            .values()
            .filter(|n| n.profile_ids.iter().any(|id| id == profile_id))
            .map(|n| n.title.clone())
            .collect())
    }
    fn load(&self) -> Result<Snapshot, String> {
        match self.tree.get(b"snapshot").map_err(|e| e.to_string())? {
            Some(bytes) => serde_json::from_slice(&bytes).map_err(|_| "invalid notes store".into()),
            None => Ok(Snapshot::default()),
        }
    }
}
fn field<'a>(params: &'a Value, key: &str) -> Result<&'a Value, String> {
    params.get(key).ok_or_else(|| format!("missing {key}"))
}
fn valid_name(name: &str) -> bool {
    !name.trim().is_empty() && name.len() <= 512 && !name.contains('\0')
}
fn parent_valid(snapshot: &Snapshot, id: &str, parent: &Option<String>) -> Result<(), String> {
    let mut cursor = parent.as_deref();
    let mut visited = std::collections::HashSet::new();
    while let Some(value) = cursor {
        if value == id || !visited.insert(value) {
            return Err("folder cycle".into());
        }
        cursor = snapshot
            .folders
            .get(value)
            .ok_or("parent folder not found")?
            .parent_id
            .as_deref();
    }
    Ok(())
}
fn descendants(snapshot: &Snapshot, root: &str) -> Vec<String> {
    let mut ids = vec![root.to_string()];
    loop {
        let more: Vec<_> = snapshot
            .folders
            .values()
            .filter(|f| {
                f.parent_id.as_ref().is_some_and(|id| ids.contains(id)) && !ids.contains(&f.id)
            })
            .map(|f| f.id.clone())
            .collect();
        if more.is_empty() {
            return ids;
        }
        ids.extend(more);
    }
}
/// Returns only ordinary notes; no Vault access and no secrets in error messages.
/// `profile_ids` must be fetched from the authoritative ProfileStore for write validation.
pub fn dispatch(
    store: &NotesStore,
    method: &str,
    params: Value,
    profile_ids: &[String],
) -> Result<Value, String> {
    let _guard = store.lock.lock().map_err(|_| "notes lock unavailable")?;
    let previous = store.tree.get(b"snapshot").map_err(|e| e.to_string())?;
    let mut snapshot: Snapshot = match &previous {
        Some(bytes) => serde_json::from_slice(bytes).map_err(|_| "invalid notes store")?,
        None => Snapshot::default(),
    };
    if method == "notes.list" {
        return Ok(
            json!({"notes":snapshot.notes.values().collect::<Vec<_>>(),"folders":snapshot.folders.values().collect::<Vec<_>>(),"revision":snapshot.revision}),
        );
    }
    if method == "notes.deleteImpact" {
        let id = field(&params, "id")?.as_str().ok_or("invalid id")?;
        let ids = descendants(&snapshot, id);
        return Ok(
            json!({"notes":snapshot.notes.values().filter(|n| n.id == id || n.parent_id.as_ref().is_some_and(|p| ids.contains(p))).map(|n| json!({"id":n.id,"title":n.title})).collect::<Vec<_>>(),"folders":ids,"revision":snapshot.revision}),
        );
    }
    let expected = field(&params, "expectedVersion")?
        .as_u64()
        .ok_or("invalid expectedVersion")?;
    if params.get("expectedRevision").is_some()
        && params["expectedRevision"].as_u64() != Some(snapshot.revision)
    {
        return Err("revision conflict; draft retained".into());
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "clock unavailable")?
        .as_millis() as u64;
    let result = match method {
        "notes.put" => {
            let mut note: Note = serde_json::from_value(field(&params, "note")?.clone())
                .map_err(|_| "invalid note")?;
            if !valid_name(&note.id)
                || !valid_name(&note.title)
                || note.markdown.len() > 2 * 1024 * 1024
                || note.tags.len() > 100
                || note.tags.iter().any(|t| !valid_name(t))
                || note.profile_ids.len() > 1024
            {
                return Err("invalid note bounds".into());
            }
            if snapshot.folders.contains_key(&note.id) {
                return Err("id already exists".into());
            }
            parent_valid(&snapshot, &note.id, &note.parent_id)?;
            if note.profile_ids.iter().any(|id| {
                !profile_ids.contains(id)
                    && !snapshot
                        .notes
                        .get(&note.id)
                        .is_some_and(|old| old.profile_ids.contains(id))
            }) {
                return Err("profile reference not found".into());
            }
            if snapshot.notes.get(&note.id).map_or(0, |n| n.version) != expected {
                return Err("version conflict; draft retained".into());
            }
            note.version = snapshot.revision + 1;
            note.updated_at = now;
            let result = json!(note);
            snapshot.notes.insert(note.id.clone(), note);
            result
        }
        "notes.putFolder" => {
            let mut folder: Folder = serde_json::from_value(field(&params, "folder")?.clone())
                .map_err(|_| "invalid folder")?;
            if !valid_name(&folder.id) || !valid_name(&folder.name) {
                return Err("invalid folder".into());
            }
            if snapshot.notes.contains_key(&folder.id) {
                return Err("id already exists".into());
            }
            parent_valid(&snapshot, &folder.id, &folder.parent_id)?;
            if snapshot.folders.get(&folder.id).map_or(0, |f| f.version) != expected {
                return Err("version conflict".into());
            }
            folder.version = snapshot.revision + 1;
            folder.updated_at = now;
            let result = json!(folder);
            snapshot.folders.insert(folder.id.clone(), folder);
            result
        }
        "notes.delete" => {
            let id = field(&params, "id")?.as_str().ok_or("invalid id")?;
            let version = snapshot
                .notes
                .get(id)
                .map(|n| n.version)
                .or_else(|| snapshot.folders.get(id).map(|f| f.version))
                .ok_or("not found")?;
            if version != expected {
                return Err("version conflict".into());
            }
            if snapshot.folders.contains_key(id) {
                let ids = descendants(&snapshot, id);
                let has_children = ids.len() > 1
                    || snapshot
                        .notes
                        .values()
                        .any(|n| n.parent_id.as_ref().is_some_and(|p| ids.contains(p)));
                if has_children
                    && (params["confirmCascade"] != true
                        || params["expectedRevision"].as_u64() != Some(snapshot.revision))
                {
                    return Err("folder contains notes; inspect impact and explicitly confirm current revision".into());
                }
                snapshot
                    .notes
                    .retain(|_, n| !n.parent_id.as_ref().is_some_and(|p| ids.contains(p)));
                snapshot.folders.retain(|key, _| !ids.contains(key));
            } else {
                snapshot.notes.remove(id);
            }
            json!({"deleted":true})
        }
        _ => return Err("unknown notes method".into()),
    };
    // Persist explicit deletion intent, including cascading children. Never infer
    // deletion from an empty store on a newly joined device.
    if let Some(previous) = &previous {
        let old: Snapshot = serde_json::from_slice(previous).map_err(|_| "invalid notes store")?;
        for id in old.notes.keys().chain(old.folders.keys()) {
            if !snapshot.notes.contains_key(id) && !snapshot.folders.contains_key(id) {
                snapshot.deleted.insert(id.clone(), id.clone());
            }
        }
    }
    for id in snapshot.notes.keys().chain(snapshot.folders.keys()) {
        snapshot.deleted.remove(id);
    }
    snapshot.revision += 1;
    let bytes = serde_json::to_vec(&snapshot).map_err(|_| "notes serialization failed")?;
    store
        .tree
        .compare_and_swap(b"snapshot", previous.as_deref(), Some(bytes))
        .map_err(|e| e.to_string())?
        .map_err(|_| "version conflict; draft retained")?;
    store.tree.flush().map_err(|e| e.to_string())?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn draft_owner_blocks_apply_until_saved_and_baseline_survives_reopen() {
        let path = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        {
            let store = NotesStore::open(&path).unwrap();
            store.edit_state("editor".into(), true).unwrap();
            assert!(store.sync_paused());
            assert!(store.apply_sync(vec![], vec![], 0).is_err());
            store.edit_state("editor".into(), false).unwrap();
            assert!(!store.sync_paused());
            store.apply_sync(vec![], vec![], 0).unwrap();
            store
                .set_export_baseline(&BTreeMap::from([("n".into(), b"last exported".to_vec())]))
                .unwrap();
        }
        let store = NotesStore::open(&path).unwrap();
        assert_eq!(store.export_baseline().unwrap()["n"], b"last exported");
    }
    #[test]
    fn sync_repairs_folder_cycles_recovers_orphans_and_preserves_local_edits() {
        let store =
            NotesStore::open(std::env::temp_dir().join(uuid::Uuid::new_v4().to_string())).unwrap();
        let folder = |id: &str, parent: &str| {
            SyncEntity::Folder(Folder {
                id: id.into(),
                parent_id: Some(parent.into()),
                name: id.into(),
                recovered_parent_id: None,
                version: 0,
                updated_at: 0,
            })
        };
        store
            .apply_sync(vec![folder("a", "b"), folder("b", "a")], vec![], 0)
            .unwrap();
        let (entities, _, revision) = store.sync_snapshot().unwrap();
        assert!(
            matches!(&entities[0],SyncEntity::Folder(f) if f.id=="a" && f.parent_id.is_none() && f.recovered_parent_id.as_deref()==Some("b"))
        );
        store
            .apply_sync(vec![folder("a", "missing")], vec![], revision)
            .unwrap();
        let (entities, _, revision) = store.sync_snapshot().unwrap();
        let SyncEntity::Folder(f) = &entities[0] else {
            panic!()
        };
        assert!(f.parent_id.is_none());
        let saved=dispatch(&store,"notes.put",json!({"note":{"id":"n","parent_id":null,"title":"local","markdown":"draft saved during sync"},"expectedVersion":0}),&[]).unwrap();
        assert!(store
            .apply_sync(vec![], vec!["n".into()], revision)
            .is_err());
        assert_eq!(
            dispatch(&store, "notes.list", json!({}), &[]).unwrap()["notes"][0]["markdown"],
            saved["markdown"]
        );
        dispatch(
            &store,
            "notes.delete",
            json!({"id":"n","expectedVersion":saved["version"]}),
            &[],
        )
        .unwrap();
        assert!(store.sync_snapshot().unwrap().1.contains(&"n".into()));
    }

    #[test]
    fn mcp_expected_revision_rejects_stale_write_atomically() {
        let db = sled::Config::new().temporary(true).open().unwrap();
        let store = NotesStore::from_db(&db).unwrap();
        let note = json!({"id":"mcp","parent_id":null,"title":"MCP","markdown":"first"});
        dispatch(
            &store,
            "notes.put",
            json!({"note":note,"expectedVersion":0,"expectedRevision":0}),
            &[],
        )
        .unwrap();
        let other = json!({"id":"other","parent_id":null,"title":"Other","markdown":"stale"});
        assert!(dispatch(
            &store,
            "notes.put",
            json!({"note":other,"expectedVersion":0,"expectedRevision":0}),
            &[]
        )
        .is_err());
        assert_eq!(
            dispatch(&store, "notes.list", json!({}), &[]).unwrap()["notes"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn editing_preserves_existing_missing_profile_reference() {
        let store =
            NotesStore::open(std::env::temp_dir().join(uuid::Uuid::new_v4().to_string())).unwrap();
        let note = json!({"id":"n","parent_id":null,"title":"note","markdown":"one","profile_ids":["missing"]});
        let saved = dispatch(
            &store,
            "notes.put",
            json!({"note":note,"expectedVersion":0}),
            &["missing".into()],
        )
        .unwrap();
        let mut changed = saved.clone();
        changed["markdown"] = json!("two");
        assert!(
            dispatch(
                &store,
                "notes.put",
                json!({"note":changed,"expectedVersion":saved["version"]}),
                &[]
            )
            .is_ok(),
            "editing a synced note must retain missing profile references"
        );
    }

    #[test]
    fn persists_and_protects_profile_references() {
        let path = std::env::temp_dir().join(format!("notes-test-{}", uuid::Uuid::new_v4()));
        {
            let store = NotesStore::open(&path).unwrap();
            let note = json!({"id":"n","parent_id":null,"title":"note","markdown":"secret-like content not in errors","profile_ids":["p"]});
            assert!(dispatch(
                &store,
                "notes.put",
                json!({"note":note,"expectedVersion":0}),
                &[]
            )
            .is_err());
            dispatch(
                &store,
                "notes.put",
                json!({"note":note,"expectedVersion":0}),
                &["p".into()],
            )
            .unwrap();
            assert_eq!(store.profile_references("p").unwrap(), vec!["note"]);
        }
        let store = NotesStore::open(&path).unwrap();
        assert_eq!(
            dispatch(&store, "notes.list", json!({}), &[]).unwrap()["notes"][0]["title"],
            "note"
        );
        assert!(dispatch(
            &store,
            "notes.delete",
            json!({"id":"n","expectedVersion":0}),
            &[]
        )
        .is_err());
        drop(store);
        std::fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn cas_cycle_and_explicit_cascade() {
        let path = std::env::temp_dir().join(format!("notes-test-{}", uuid::Uuid::new_v4()));
        let store = NotesStore::open(&path).unwrap();
        let folder = dispatch(
            &store,
            "notes.putFolder",
            json!({"folder":{"id":"a","parent_id":null,"name":"A"},"expectedVersion":0}),
            &[],
        )
        .unwrap();
        assert_eq!(folder["version"], 1);
        assert!(dispatch(
            &store,
            "notes.putFolder",
            json!({"folder":{"id":"a","parent_id":"a","name":"A"},"expectedVersion":1}),
            &[]
        )
        .is_err());
        let note = json!({"id":"n","parent_id":"a","title":"N","markdown":"body","tags":[],"profile_ids":[]});
        dispatch(
            &store,
            "notes.put",
            json!({"note":note,"expectedVersion":0}),
            &[],
        )
        .unwrap();
        assert!(dispatch(
            &store,
            "notes.put",
            json!({"note":note,"expectedVersion":0}),
            &[]
        )
        .is_err());
        assert!(dispatch(
            &store,
            "notes.delete",
            json!({"id":"a","expectedVersion":1}),
            &[]
        )
        .is_err());
        dispatch(
            &store,
            "notes.delete",
            json!({"id":"a","expectedVersion":1,"confirmCascade":true,"expectedRevision":2}),
            &[],
        )
        .unwrap();
        assert_eq!(
            dispatch(&store, "notes.list", json!({}), &[]).unwrap()["notes"],
            json!([])
        );
        drop(store);
        std::fs::remove_dir_all(path).unwrap();
    }
}
