//! Connection profile importers (WindTerm, OpenSSH config, CSV, …).

mod common;
mod csv_import;
mod mobaxterm;
mod openssh;
mod putty;
mod securecrt;
mod tabby;
mod termius;
mod types;
mod windterm;
mod xshell;

pub use common::apply_ssh_import_overrides;
pub use csv_import::{detect_csv_paths, preview_csv, read_csv_file};
pub use mobaxterm::{detect_mobaxterm_paths, preview_mobaxterm, read_mobaxterm_file};
pub use openssh::{detect_openssh_paths, preview_openssh, read_openssh_file};
pub use putty::{detect_putty_paths, preview_putty, read_putty_file};
pub use securecrt::{detect_securecrt_paths, preview_securecrt, read_securecrt_file};
pub use tabby::{detect_tabby_paths, preview_tabby, read_tabby_file};
pub use termius::{detect_termius_paths, preview_termius, read_termius_file};
pub use types::{
    endpoint_key, mark_duplicates, preview_stats, ImportApplyResult, ImportCandidate,
    ImportCandidateStatus, ImportDetectPath, ImportDetectResult, ImportPreviewResult,
    ImportPreviewStats,
};
pub use windterm::{detect_windterm_paths, preview_windterm, read_windterm_file};
pub use xshell::{detect_xshell_paths, preview_xshell_at};

use std::path::PathBuf;

pub fn import_detect(source: &str) -> Result<ImportDetectResult, String> {
    Ok(match source {
        "windterm" => detect_windterm_paths(),
        "ssh-config" => detect_openssh_paths(),
        "csv" => detect_csv_paths(),
        "putty" => detect_putty_paths(),
        "mobaxterm" => detect_mobaxterm_paths(),
        "xshell" => detect_xshell_paths(),
        "securecrt" => detect_securecrt_paths(),
        "tabby" => detect_tabby_paths(),
        "termius" => detect_termius_paths(),
        other => return Err(format!("unknown import source: {other}")),
    })
}

pub fn resolve_import_path(source: &str, path: Option<&str>) -> Result<PathBuf, String> {
    if let Some(p) = path {
        return Ok(PathBuf::from(p));
    }
    match source {
        "windterm" => detect_windterm_paths()
            .paths
            .first()
            .map(|p| PathBuf::from(&p.path))
            .ok_or_else(|| "WindTerm user.sessions not found; pick a file".into()),
        "ssh-config" => {
            ssh_config_default_file().ok_or_else(|| "~/.ssh/config not found; pick a file".into())
        }
        "mobaxterm" => detect_mobaxterm_paths()
            .paths
            .first()
            .map(|p| PathBuf::from(&p.path))
            .ok_or_else(|| "MobaXterm.ini not found; pick a file".into()),
        "xshell" => detect_xshell_paths()
            .paths
            .first()
            .map(|p| PathBuf::from(&p.path))
            .ok_or_else(|| {
                "Xshell Sessions folder not found; browse for a folder or .xsh file".into()
            }),
        "tabby" => detect_tabby_paths()
            .paths
            .first()
            .map(|p| PathBuf::from(&p.path))
            .ok_or_else(|| "Tabby config.yaml not found; pick a JSON or YAML export".into()),
        "csv" | "putty" | "securecrt" | "termius" => {
            Err(format!("{source} import requires a file path"))
        }
        other => Err(format!("unknown import source: {other}")),
    }
}

pub fn read_import_text(source: &str, path: Option<&str>) -> Result<String, String> {
    let resolved = resolve_import_path(source, path)?;
    match source {
        "windterm" => read_windterm_file(resolved.as_path()),
        "ssh-config" => read_openssh_file(resolved.as_path()),
        "csv" => read_csv_file(resolved.as_path()),
        "putty" => read_putty_file(resolved.as_path()),
        "mobaxterm" => read_mobaxterm_file(resolved.as_path()),
        "securecrt" => read_securecrt_file(resolved.as_path()),
        "tabby" => read_tabby_file(resolved.as_path()),
        "termius" => read_termius_file(resolved.as_path()),
        "xshell" => Err("Xshell import reads session files from a path".into()),
        other => Err(format!("unknown import source: {other}")),
    }
}

pub fn preview_import(
    source: &str,
    text: &str,
    path: Option<&str>,
) -> Result<ImportPreviewResult, String> {
    match source {
        "windterm" => preview_windterm(text, path),
        "ssh-config" => preview_openssh(text, path),
        "csv" => preview_csv(text, path),
        "putty" => preview_putty(text, path),
        "mobaxterm" => preview_mobaxterm(text, path),
        "securecrt" => preview_securecrt(text, path),
        "tabby" => preview_tabby(text, path),
        "termius" => preview_termius(text, path),
        "xshell" => {
            let resolved = resolve_import_path(source, path)?;
            preview_xshell_at(resolved.as_path())
        }
        other => Err(format!("unknown import source: {other}")),
    }
}

pub fn load_import_preview(
    source: &str,
    path: Option<&str>,
) -> Result<ImportPreviewResult, String> {
    if source == "xshell" {
        let resolved = resolve_import_path(source, path)?;
        return preview_xshell_at(resolved.as_path());
    }
    let text = read_import_text(source, path)?;
    preview_import(source, &text, path)
}

fn ssh_config_default_file() -> Option<PathBuf> {
    crate::ssh_config::default_config_path().filter(|p| p.is_file())
}
