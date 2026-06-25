use std::path::{Path, PathBuf};

use serde_json::Value;
use uuid::Uuid;

use crate::profile::{Profile, ProfileKind, RemoteDesktopSpec};
use crate::ssh::{AuthMethod, SshProfile};

use super::types::{
    ImportCandidate, ImportCandidateStatus, ImportDetectPath, ImportDetectResult,
    ImportPreviewResult,
};

const SOURCE: &str = "windterm";
const IMPORT_TAG: &str = "import:windterm";

pub fn detect_windterm_paths() -> ImportDetectResult {
    let mut paths = Vec::new();
    for candidate in windterm_path_candidates() {
        if !candidate.is_file() {
            continue;
        }
        let label = candidate
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("user.sessions")
            .to_string();
        paths.push(ImportDetectPath {
            path: candidate.display().to_string(),
            label: format!("{} ({})", label, parent_hint(&candidate)),
        });
    }
    ImportDetectResult { paths }
}

fn parent_hint(path: &Path) -> String {
    path.parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("WindTerm")
        .to_string()
}

fn windterm_path_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let profile_dirs = ["default.v10", "default.v11", "default.v12", "default"];
    let terminal_paths = [("terminal", "user.sessions"), ("", "user.sessions")];

    if let Some(home) = home_dir() {
        for ver in profile_dirs {
            for (dir, file) in terminal_paths {
                let mut p = home.join(".wind").join("profiles").join(ver);
                if !dir.is_empty() {
                    p = p.join(dir);
                }
                out.push(p.join(file));
            }
        }
    }

    if let Ok(appdata) = std::env::var("APPDATA") {
        for ver in profile_dirs {
            for (dir, file) in terminal_paths {
                let mut p = PathBuf::from(&appdata)
                    .join("WindTerm")
                    .join("profiles")
                    .join(ver);
                if !dir.is_empty() {
                    p = p.join(dir);
                }
                out.push(p.join(file));
            }
        }
    }

    for base in ["/opt/WindTerm/profiles", "/usr/lib/WindTerm/profiles"] {
        for ver in profile_dirs {
            for (dir, file) in terminal_paths {
                let mut p = PathBuf::from(base).join(ver);
                if !dir.is_empty() {
                    p = p.join(dir);
                }
                out.push(p.join(file));
            }
        }
    }

    if let Some(home) = home_dir() {
        out.push(home.join(
            "Library/Application Support/WindTerm/profiles/default.v10/terminal/user.sessions",
        ));
    }

    out.sort();
    out.dedup();
    out
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

pub fn read_windterm_file(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))
}

pub fn preview_windterm(text: &str, path: Option<&str>) -> Result<ImportPreviewResult, String> {
    let objects = parse_session_objects(text)?;
    let candidates = objects
        .into_iter()
        .filter_map(|obj| map_windterm_object(&obj))
        .collect::<Vec<_>>();
    Ok(ImportPreviewResult {
        source: SOURCE.into(),
        path: path.map(String::from),
        stats: super::types::preview_stats(&candidates),
        candidates,
    })
}

fn parse_session_objects(text: &str) -> Result<Vec<Value>, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("WindTerm sessions file is empty".into());
    }
    if trimmed.starts_with('[') {
        serde_json::from_str(trimmed).map_err(|e| format!("invalid WindTerm JSON array: {e}"))
    } else {
        let mut out = Vec::new();
        for (i, raw) in trimmed.lines().enumerate() {
            let line = raw.trim().trim_end_matches(',');
            if line.is_empty() {
                continue;
            }
            let v: Value = serde_json::from_str(line)
                .map_err(|e| format!("invalid WindTerm session line {}: {e}", i + 1))?;
            out.push(v);
        }
        if out.is_empty() {
            return Err("no WindTerm session objects found".into());
        }
        Ok(out)
    }
}

fn map_windterm_object(obj: &Value) -> Option<ImportCandidate> {
    let map = obj.as_object()?;
    let source_id = str_field(map, "session.uuid")
        .or_else(|| str_field(map, "session.label"))
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let name = str_field(map, "session.label")
        .or_else(|| str_field(map, "session.target"))
        .unwrap_or_else(|| "Unnamed".into());
    let group = str_field(map, "session.group").map(normalize_group);
    let protocol = str_field(map, "session.protocol").unwrap_or_else(|| "SSH".into());
    let port = map
        .get("session.port")
        .and_then(|v| v.as_u64())
        .map(|p| p as u16)
        .unwrap_or(22);
    let target = str_field(map, "session.target").unwrap_or_default();
    let (user, host) = parse_target(&target);

    let mut warnings = Vec::new();
    if str_field(map, "ssh.password").is_some() || str_field(map, "session.password").is_some() {
        warnings.push("password not imported; using ssh-agent".into());
    }

    let tags = vec![IMPORT_TAG.to_string()];

    let (kind, profile, error_message) = match protocol.to_ascii_uppercase().as_str() {
        "SSH" | "SFTP" => {
            if host.is_empty() {
                (
                    "error".into(),
                    None,
                    Some("missing session.target host".into()),
                )
            } else {
                let auth = identity_from_map(map).unwrap_or(AuthMethod::Agent);
                let ssh = SshProfile {
                    host: host.clone(),
                    port: if port == 0 { 22 } else { port },
                    user: if user.is_empty() {
                        "root".into()
                    } else {
                        user.clone()
                    },
                    auth,
                    jump_via: vec![],
                };
                (
                    "ssh".into(),
                    Some(build_profile(
                        name.clone(),
                        group.clone(),
                        tags.clone(),
                        ProfileKind::Ssh { ssh },
                    )),
                    None,
                )
            }
        }
        "RDP" => {
            if host.is_empty() {
                (
                    "error".into(),
                    None,
                    Some("missing session.target host".into()),
                )
            } else {
                let rdp = RemoteDesktopSpec {
                    host: host.clone(),
                    port: if port == 0 { 3389 } else { port },
                    ssh_profile_id: None,
                    local_bind_port: 0,
                };
                (
                    "rdp".into(),
                    Some(build_profile(
                        name.clone(),
                        group.clone(),
                        tags.clone(),
                        ProfileKind::Rdp { rdp },
                    )),
                    None,
                )
            }
        }
        "VNC" => {
            if host.is_empty() {
                (
                    "error".into(),
                    None,
                    Some("missing session.target host".into()),
                )
            } else {
                let spec = RemoteDesktopSpec {
                    host: host.clone(),
                    port: if port == 0 { 5900 } else { port },
                    ssh_profile_id: None,
                    local_bind_port: 0,
                };
                (
                    "vnc".into(),
                    Some(build_profile(
                        name.clone(),
                        group.clone(),
                        tags.clone(),
                        ProfileKind::Vnc { spec },
                    )),
                    None,
                )
            }
        }
        other => (
            "error".into(),
            None,
            Some(format!("unsupported WindTerm protocol: {other}")),
        ),
    };

    let status = if error_message.is_some() {
        ImportCandidateStatus::Error
    } else {
        ImportCandidateStatus::Ready
    };

    Some(ImportCandidate {
        source_id,
        source: SOURCE.into(),
        name,
        group,
        tags,
        note: None,
        kind,
        status,
        warnings,
        error_message,
        duplicate_of: None,
        profile,
    })
}

fn build_profile(
    name: String,
    group: Option<String>,
    tags: Vec<String>,
    spec: ProfileKind,
) -> Profile {
    Profile {
        schema_version: 1,
        id: Uuid::new_v4(),
        name,
        group,
        tags,
        note: Some("Imported from WindTerm".into()),
        icon: None,
        favorite: false,
        spec,
    }
}

fn identity_from_map(map: &serde_json::Map<String, Value>) -> Option<AuthMethod> {
    for key in [
        "ssh.identityFile",
        "ssh.identity.file",
        "session.identityFile",
        "ssh.privateKey",
    ] {
        if let Some(path) = str_field(map, key) {
            if !path.is_empty() {
                return Some(AuthMethod::PublicKey {
                    key_path: PathBuf::from(path),
                    passphrase: None,
                });
            }
        }
    }
    None
}

fn str_field(map: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    map.get(key).and_then(|v| {
        if let Some(s) = v.as_str() {
            if s.is_empty() {
                None
            } else {
                Some(s.to_string())
            }
        } else {
            v.as_u64().map(|n| n.to_string())
        }
    })
}

fn parse_target(target: &str) -> (String, String) {
    let t = target.trim();
    if t.is_empty() {
        return (String::new(), String::new());
    }
    if let Some((user, host)) = t.split_once('@') {
        return (user.trim().to_string(), host.trim().to_string());
    }
    (String::new(), t.to_string())
}

fn normalize_group(g: String) -> String {
    g.trim().trim_start_matches('/').replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"[
      {
        "session.group": "Production",
        "session.label": "web-01",
        "session.port": 2222,
        "session.protocol": "SSH",
        "session.target": "alice@10.0.0.5",
        "session.uuid": "uuid-1",
        "ssh.identityFile": "/home/alice/.ssh/id_ed25519"
      },
      {
        "session.label": "win-rdp",
        "session.port": 3389,
        "session.protocol": "RDP",
        "session.target": "10.0.0.8",
        "session.uuid": "uuid-2"
      },
      {
        "session.label": "telnet-old",
        "session.protocol": "Telnet",
        "session.target": "1.2.3.4",
        "session.uuid": "uuid-3"
      }
    ]"#;

    #[test]
    fn parses_array_and_maps_ssh_rdp_error() {
        let preview = preview_windterm(SAMPLE, None).unwrap();
        assert_eq!(preview.candidates.len(), 3);
        assert_eq!(preview.stats.ready, 2);
        assert_eq!(preview.stats.error, 1);
        let ssh = preview
            .candidates
            .iter()
            .find(|c| c.source_id == "uuid-1")
            .unwrap();
        assert_eq!(ssh.kind, "ssh");
        assert!(ssh.profile.is_some());
        let p = ssh.profile.as_ref().unwrap();
        if let ProfileKind::Ssh { ssh } = &p.spec {
            assert_eq!(ssh.host, "10.0.0.5");
            assert_eq!(ssh.port, 2222);
            assert_eq!(ssh.user, "alice");
        } else {
            panic!("expected ssh");
        }
        assert!(ssh.tags.contains(&"import:windterm".to_string()));
    }

    #[test]
    fn parses_line_delimited_objects() {
        let text = r#"{"session.label":"a","session.protocol":"SSH","session.target":"h@1.1.1.1","session.uuid":"u1"}
{"session.label":"b","session.protocol":"SSH","session.target":"u@2.2.2.2","session.uuid":"u2"}"#;
        let preview = preview_windterm(text, None).unwrap();
        assert_eq!(preview.candidates.len(), 2);
    }
}
