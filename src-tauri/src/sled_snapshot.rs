//! Copy a sled database directory/file into a per-process temp path so a
//! second AeroTab instance can read profiles/settings while the primary holds the lock.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

fn copy_entry(src: &Path, dest: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(src)?;
    if meta.is_dir() {
        fs::create_dir_all(dest)?;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            copy_entry(&entry.path(), &dest.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(src, dest)?;
        Ok(())
    }
}

/// Deep-copy `src` sled data to a fresh temp directory for read-only secondary instances.
pub fn temp_snapshot(src: &Path, label: &str) -> io::Result<PathBuf> {
    let dest = std::env::temp_dir().join(format!(
        "aerotab-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    if dest.exists() {
        let _ = fs::remove_dir_all(&dest);
    }
    if src.exists() {
        copy_entry(src, &dest)?;
    } else {
        fs::create_dir_all(&dest)?;
    }
    Ok(dest)
}
