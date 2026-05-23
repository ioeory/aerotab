//! Session manager: owns tabs, panes, and their lifecycle.
//!
//! Real implementation lands in W3-W4. This stub only fixes the public
//! shape that the IPC layer will depend on.

use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct SessionId(pub Uuid);

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum SessionKind {
    LocalShell,
    Ssh,
    Serial,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionMeta {
    pub id: SessionId,
    pub kind: SessionKind,
    pub title: String,
}

#[derive(Default)]
pub struct SessionManager {
    sessions: HashMap<SessionId, SessionMeta>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn list(&self) -> Vec<SessionMeta> {
        self.sessions.values().cloned().collect()
    }

    /// Creates a placeholder session record. Backing I/O is wired in W4.
    pub fn open(&mut self, kind: SessionKind, title: impl Into<String>) -> SessionMeta {
        let meta = SessionMeta {
            id: SessionId(Uuid::new_v4()),
            kind,
            title: title.into(),
        };
        self.sessions.insert(meta.id, meta.clone());
        meta
    }

    pub fn close(&mut self, id: SessionId) -> bool {
        self.sessions.remove(&id).is_some()
    }
}
