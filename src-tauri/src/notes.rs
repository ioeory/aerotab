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
}
#[derive(Clone, Default, Serialize, Deserialize)]
struct Snapshot {
    notes: BTreeMap<String, Note>,
    folders: BTreeMap<String, Folder>,
    revision: u64,
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
        })
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
            if note.profile_ids.iter().any(|id| !profile_ids.contains(id)) {
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
