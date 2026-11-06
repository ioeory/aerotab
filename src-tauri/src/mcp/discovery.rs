//! Small non-secret desktop endpoint descriptor used by the stdio bridge.
use std::path::{Path, PathBuf};

pub fn data_dir() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("AEROTAB_DATA_DIR").filter(|s| !s.is_empty()) {
        return Some(path.into());
    }
    #[cfg(windows)]
    let root = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let root = std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join("Library/Application Support"));
    #[cfg(not(any(windows, target_os = "macos")))]
    let root = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")));
    root.map(|path| path.join("com.aerotab"))
}

/// The descriptor contains no token; ownership checks prevent one desktop
/// instance from removing the descriptor of another running instance.
pub fn publish(dir: &Path, url: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let target = dir.join("mcp-endpoint.json");
    let temporary = dir.join(format!("mcp-endpoint-{}.tmp", std::process::id()));
    let value = serde_json::json!({"url":url,"pid":std::process::id()});
    std::fs::write(&temporary, serde_json::to_vec(&value)?)?;
    // Windows rename cannot replace existing files.
    #[cfg(windows)]
    if target.exists() {
        std::fs::remove_file(&target)?;
    }
    std::fs::rename(temporary, target)
}

pub fn remove_owned(dir: &Path) {
    let target = dir.join("mcp-endpoint.json");
    if let Ok(bytes) = std::fs::read(&target) {
        if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) {
            if value["pid"].as_u64() == Some(u64::from(std::process::id())) {
                let _ = std::fs::remove_file(target);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn descriptor_is_non_secret_and_only_owner_removes_it() {
        let dir = std::env::temp_dir().join(format!("mcp-discovery-{}", uuid::Uuid::new_v4()));
        publish(&dir, "http://127.0.0.1:12345/mcp").unwrap();
        let target = dir.join("mcp-endpoint.json");
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&target).unwrap()).unwrap();
        assert_eq!(value.as_object().unwrap().len(), 2);
        assert_eq!(value["pid"], std::process::id());
        remove_owned(&dir);
        assert!(!target.exists());
        std::fs::write(&target, br#"{"url":"http://127.0.0.1:1/mcp","pid":0}"#).unwrap();
        remove_owned(&dir);
        assert!(target.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
