use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::profile::ProfileKind;
use crate::ssh::{AuthMethod, SshProfile};

use super::common::{build_profile, error_candidate, home_dir, read_text_file, ready_candidate};
use super::types::{ImportCandidate, ImportDetectPath, ImportDetectResult, ImportPreviewResult};

const SOURCE: &str = "xshell";
const IMPORT_TAG: &str = "import:xshell";
const MAX_XSH_FILES: usize = 2_000;

pub fn detect_xshell_paths() -> ImportDetectResult {
    let mut paths = Vec::new();
    if let Some(home) = home_dir() {
        let docs = home.join("Documents");
        if docs.is_dir() {
            collect_xshell_dirs(&docs, &mut paths);
        }
        let legacy = docs.join("NetSarang").join("Xshell").join("Sessions");
        push_dir_path(&mut paths, &legacy, "Xshell Sessions");
    }
    ImportDetectResult { paths }
}

fn collect_xshell_dirs(root: &Path, out: &mut Vec<ImportDetectPath>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.contains("NetSarang") {
            find_sessions_subdirs(&path, out, 0);
        }
    }
}

fn find_sessions_subdirs(root: &Path, out: &mut Vec<ImportDetectPath>, depth: usize) {
    if depth > 5 {
        return;
    }
    let sessions = root.join("Xshell").join("Sessions");
    if sessions.is_dir() {
        push_dir_path(out, &sessions, "Xshell Sessions");
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.path().is_dir() {
            find_sessions_subdirs(&entry.path(), out, depth + 1);
        }
    }
}

fn push_dir_path(out: &mut Vec<ImportDetectPath>, path: &Path, label: &str) {
    if !path.is_dir() {
        return;
    }
    out.push(ImportDetectPath {
        path: path.display().to_string(),
        label: label.into(),
    });
}

pub fn preview_xshell_at(path: &Path) -> Result<ImportPreviewResult, String> {
    let files = collect_xsh_files(path)?;
    if files.is_empty() {
        return Err("no .xsh session files found".into());
    }
    let sessions_root = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent().unwrap_or(path).to_path_buf()
    };
    let candidates = files
        .into_iter()
        .filter_map(|file| {
            let text = read_text_file(&file).ok()?;
            Some(map_xsh_file(&file, &sessions_root, &text))
        })
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return Err("no readable Xshell session files found".into());
    }
    Ok(ImportPreviewResult {
        source: SOURCE.into(),
        path: Some(path.display().to_string()),
        stats: super::types::preview_stats(&candidates),
        candidates,
    })
}

fn collect_xsh_files(path: &Path) -> Result<Vec<PathBuf>, String> {
    if path.is_file() {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if ext == "xsh" {
            return Ok(vec![path.to_path_buf()]);
        }
        return Err(format!("expected .xsh file, got {}", path.display()));
    }
    if !path.is_dir() {
        return Err(format!("path not found: {}", path.display()));
    }
    let mut files = Vec::new();
    collect_xsh_recursive(path, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_xsh_recursive(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    if out.len() >= MAX_XSH_FILES {
        return Err(format!("too many .xsh files (limit {MAX_XSH_FILES})"));
    }
    let entries = std::fs::read_dir(dir).map_err(|e| format!("read dir {}: {e}", dir.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_xsh_recursive(&path, out)?;
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("xsh"))
        {
            out.push(path);
            if out.len() >= MAX_XSH_FILES {
                return Err(format!("too many .xsh files (limit {MAX_XSH_FILES})"));
            }
        }
    }
    Ok(())
}

fn map_xsh_file(path: &Path, sessions_root: &Path, text: &str) -> ImportCandidate {
    let source_id = path.display().to_string();
    let tags = vec![IMPORT_TAG.to_string()];
    let default_name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("session")
        .to_string();

    let fields = parse_xsh_fields(text);
    if fields.is_empty() || looks_binary(text) {
        return error_candidate(
            SOURCE,
            source_id,
            default_name,
            "unreadable or encrypted Xshell session file".into(),
            tags,
        );
    }

    let name = field(&fields, &["name", "description", "sessionname"]).unwrap_or(default_name);
    let protocol = field(&fields, &["protocol", "type"])
        .unwrap_or_else(|| "ssh".into())
        .to_ascii_lowercase();

    if !matches!(protocol.as_str(), "ssh" | "sftp" | "") {
        return error_candidate(
            SOURCE,
            source_id,
            name,
            format!("unsupported Xshell protocol: {protocol}"),
            tags,
        );
    }

    let Some(host) = field(&fields, &["host", "hostname"]) else {
        return error_candidate(SOURCE, source_id, name, "missing Host".into(), tags);
    };
    let port = field(&fields, &["port"])
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(22);
    let user = field(&fields, &["username", "user"]).unwrap_or_else(|| "root".into());

    let mut warnings = vec!["password not imported; using ssh-agent".into()];
    if field(&fields, &["username", "user"]).is_none() {
        warnings.push("no UserName; defaulting to root".into());
    }

    let auth = field(&fields, &["userkey", "publickey", "identityfile"])
        .map(|key| AuthMethod::PublicKey {
            key_path: PathBuf::from(key),
            passphrase: None,
        })
        .unwrap_or(AuthMethod::Agent);

    let group = group_from_path(sessions_root, path);
    let ssh = SshProfile {
        host,
        port,
        user,
        auth,
        jump_via: vec![],
    };
    let profile = build_profile(
        name.clone(),
        group.clone(),
        tags.clone(),
        Some("Imported from Xshell".into()),
        ProfileKind::Ssh { ssh },
    );
    ready_candidate(
        SOURCE,
        source_id,
        name,
        group,
        tags,
        Some("Imported from Xshell".into()),
        "ssh",
        profile,
        warnings,
    )
}

fn looks_binary(text: &str) -> bool {
    text.bytes().filter(|b| *b == 0).take(3).count() >= 1
}

fn parse_xsh_fields(text: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        out.insert(key.trim().to_ascii_lowercase(), value.trim().to_string());
    }
    out
}

fn field(fields: &HashMap<String, String>, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(v) = fields.get(*key) {
            if !v.is_empty() {
                return Some(v.clone());
            }
        }
    }
    None
}

fn group_from_path(sessions_root: &Path, file: &Path) -> Option<String> {
    let parent = file.parent()?;
    let rel = parent.strip_prefix(sessions_root).ok()?;
    if rel.as_os_str().is_empty() {
        return None;
    }
    Some(rel.to_string_lossy().replace('\\', "/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
[Session]
Host=10.0.0.8
Port=2222
UserName=dev
Protocol=SSH
Description=build host
"#;

    #[test]
    fn parses_ini_like_xsh() {
        let path = PathBuf::from("/tmp/Sessions/Prod/web.xsh");
        let candidate = map_xsh_file(&path, Path::new("/tmp/Sessions"), SAMPLE);
        assert_eq!(candidate.name, "build host");
        assert_eq!(candidate.group.as_deref(), Some("Prod"));
        assert_eq!(candidate.kind, "ssh");
    }

    #[test]
    fn rejects_non_ssh_protocol() {
        let text = "Host=1.1.1.1\nPort=22\nProtocol=RDP\n";
        let candidate = map_xsh_file(Path::new("/tmp/a.xsh"), Path::new("/tmp"), text);
        assert_eq!(
            candidate.status,
            super::super::types::ImportCandidateStatus::Error
        );
    }
}
