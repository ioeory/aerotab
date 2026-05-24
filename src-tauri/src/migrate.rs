//! One-time migration from Tabby v2 (`org.tabby.v2`) app data to AeroTab (`com.aerotab`).

use std::fs;
use std::path::{Path, PathBuf};

const MARKER: &str = ".migrated-from-org.tabby.v2";

fn legacy_data_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(home) = std::env::var("HOME") {
        let base = PathBuf::from(home).join(".local/share");
        dirs.push(base.join("org.tabby.v2"));
    }
    if let Ok(appdata) = std::env::var("APPDATA") {
        dirs.push(PathBuf::from(appdata).join("org.tabby.v2"));
    }
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        dirs.push(PathBuf::from(xdg).join("org.tabby.v2"));
    }
    dirs
}

fn dir_has_user_data(dir: &Path) -> bool {
    dir.join("profiles.sled").exists()
        || dir.join("settings").exists()
        || dir.join("vault").exists()
}

fn copy_file_if_present(src_dir: &Path, dst_dir: &Path, name: &str) {
    let from = src_dir.join(name);
    let to = dst_dir.join(name);
    if from.is_file() && !to.exists() {
        if let Err(e) = fs::copy(&from, &to) {
            tracing::warn!(file = name, error = %e, "migration copy failed");
        }
    }
}

fn copy_tree_file(src_dir: &Path, dst_dir: &Path, rel: &str) {
    let from = src_dir.join(rel);
    let to = dst_dir.join(rel);
    if from.is_file() && !to.exists() {
        if let Some(parent) = to.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Err(e) = fs::copy(&from, &to) {
            tracing::warn!(file = rel, error = %e, "migration copy failed");
        }
    }
}

/// Copy legacy stores into `new_dir` when the new directory is empty and a legacy tree exists.
pub fn migrate_app_data_if_needed(new_dir: &Path) {
    if new_dir.join(MARKER).exists() {
        return;
    }
    if dir_has_user_data(new_dir) {
        return;
    }
    for legacy in legacy_data_dirs() {
        if !legacy.is_dir() || !dir_has_user_data(&legacy) {
            continue;
        }
        let _ = fs::create_dir_all(new_dir);
        for name in ["profiles.sled", "known_hosts", "plugins"] {
            copy_file_if_present(&legacy, new_dir, name);
        }
        copy_tree_file(&legacy, new_dir, "settings");
        copy_tree_file(&legacy, new_dir, "vault");
        let _ = fs::write(new_dir.join(MARKER), legacy.display().to_string());
        tracing::info!(
            from = %legacy.display(),
            to = %new_dir.display(),
            "migrated legacy Tabby v2 app data"
        );
        return;
    }
}
