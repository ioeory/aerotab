use std::path::{Path, PathBuf};

use serde_json::Value;
use uuid::Uuid;

use crate::profile::{Profile, ProfileKind, RemoteDesktopSpec};
use crate::ssh::keys::{
    expand_identity_path, first_existing_ssh_key, resolve_existing_identity_file,
};
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
    if windterm_has_saved_login(map) {
        warnings.push(
            "WindTerm saved login (password/key) is encrypted and not imported; configure auth in AeroTab if needed".into(),
        );
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
                let auth = auth_from_windterm(map, &mut warnings);
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

fn windterm_identity_keys() -> &'static [&'static str] {
    #[cfg(windows)]
    {
        &[
            "ssh.identityFilePath.windows",
            "ssh.identityFilePath",
            "ssh.identityFile",
            "ssh.identity.file",
            "session.identityFilePath",
            "session.identityfilePath",
            "session.identityFile",
            "ssh.privateKey",
            "ssh.identityFilePath.linux",
            "ssh.identityFilePath.macos",
        ]
    }
    #[cfg(target_os = "macos")]
    {
        &[
            "ssh.identityFilePath.macos",
            "ssh.identityFilePath",
            "ssh.identityFile",
            "ssh.identity.file",
            "session.identityFilePath",
            "session.identityfilePath",
            "session.identityFile",
            "ssh.privateKey",
            "ssh.identityFilePath.windows",
            "ssh.identityFilePath.linux",
        ]
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        &[
            "ssh.identityFilePath.linux",
            "ssh.identityFilePath",
            "ssh.identityFile",
            "ssh.identity.file",
            "session.identityFilePath",
            "session.identityfilePath",
            "session.identityFile",
            "ssh.privateKey",
            "ssh.identityFilePath.windows",
            "ssh.identityFilePath.macos",
        ]
    }
    #[cfg(not(any(windows, unix)))]
    {
        &[
            "ssh.identityFilePath",
            "ssh.identityFile",
            "ssh.identity.file",
            "session.identityFilePath",
            "session.identityfilePath",
            "session.identityFile",
            "ssh.privateKey",
        ]
    }
}

fn windterm_has_saved_login(map: &serde_json::Map<String, Value>) -> bool {
    for key in [
        "session.autoLogin",
        "ssh.password",
        "session.password",
        "ssh.passphrase",
    ] {
        if let Some(v) = map.get(key) {
            if v.as_str().is_some_and(|s| !s.is_empty()) {
                return true;
            }
        }
    }
    false
}

fn auth_from_windterm(
    map: &serde_json::Map<String, Value>,
    warnings: &mut Vec<String>,
) -> AuthMethod {
    let (configured, missing) = identity_from_map(map);
    if let Some(path) = configured {
        return AuthMethod::PublicKey {
            key_path: path,
            passphrase: None,
        };
    }
    let had_missing = missing.is_some();
    if let Some(missing) = missing {
        warnings.push(format!(
            "WindTerm identity file not found ({missing}); using local default key or ssh-agent"
        ));
    }
    if let Some(key) = first_existing_ssh_key() {
        if !had_missing {
            warnings.push(format!(
                "no identity file in WindTerm session; using default key {}",
                key.display()
            ));
        }
        return AuthMethod::PublicKey {
            key_path: key,
            passphrase: None,
        };
    }
    warnings.push(
        "no usable private key; will try ssh-agent at connect (start OpenSSH Authentication Agent on Windows)".into(),
    );
    AuthMethod::Agent
}

/// First existing identity file from WindTerm fields; `missing` describes the last configured path tried.
fn identity_from_map(
    map: &serde_json::Map<String, Value>,
) -> (Option<std::path::PathBuf>, Option<String>) {
    let mut missing: Option<String> = None;
    for key in windterm_identity_keys() {
        if let Some(raw) = str_field(map, key) {
            if raw.is_empty() {
                continue;
            }
            if let Some(path) = resolve_existing_identity_file(&raw) {
                return (Some(path), None);
            }
            let expanded = expand_identity_path(&raw);
            missing = Some(format!("{key}={raw} -> {}", expanded.display()));
        }
    }
    for (key, value) in map {
        if windterm_identity_keys().contains(&key.as_str()) {
            continue;
        }
        let Some(raw) = value.as_str() else {
            continue;
        };
        if raw.is_empty() {
            continue;
        }
        let lower = key.to_ascii_lowercase();
        if !["identity", "privatekey"]
            .iter()
            .any(|needle| lower.contains(needle))
        {
            continue;
        }
        if let Some(path) = resolve_existing_identity_file(raw) {
            return (Some(path), None);
        }
        let expanded = expand_identity_path(raw);
        missing = Some(format!("{key}={raw} -> {}", expanded.display()));
    }
    (None, missing)
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
    g.trim().trim_start_matches('/').replace(['\\', '>'], "/")
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
            if resolve_existing_identity_file("/home/alice/.ssh/id_ed25519").is_some() {
                assert!(matches!(ssh.auth, AuthMethod::PublicKey { .. }));
            } else {
                assert!(matches!(
                    ssh.auth,
                    AuthMethod::PublicKey { .. } | AuthMethod::Agent
                ));
            }
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

    #[test]
    fn normalizes_gt_group_separator() {
        let text = r#"[{
            "session.group": "Doocom>Local-Dev",
            "session.label": "vm-1",
            "session.protocol": "SSH",
            "session.target": "u@1.2.3.4",
            "session.uuid": "g1"
        }]"#;
        let preview = preview_windterm(text, None).unwrap();
        assert_eq!(
            preview.candidates[0].group.as_deref(),
            Some("Doocom/Local-Dev")
        );
    }

    #[test]
    fn maps_identity_file_path_windows_field() {
        let Some(home) = crate::ssh::keys::ssh_home_dir() else {
            return;
        };
        let key = home.join(".ssh/id_ed25519");
        if !key.is_file() {
            return;
        }
        let text = r#"[{
            "session.label": "nginx",
            "session.protocol": "SSH",
            "session.target": "root@192.168.1.106",
            "session.uuid": "w1",
            "ssh.identityFilePath.windows": "%USERPROFILE%\\.ssh\\id_ed25519"
        }]"#;
        let preview = preview_windterm(text, None).unwrap();
        let p = preview.candidates[0].profile.as_ref().unwrap();
        if let ProfileKind::Ssh { ssh } = &p.spec {
            assert!(matches!(ssh.auth, AuthMethod::PublicKey { .. }));
            if let AuthMethod::PublicKey { key_path, .. } = &ssh.auth {
                assert_eq!(key_path, &key);
            }
        } else {
            panic!("expected ssh");
        }
    }

    #[test]
    fn missing_windterm_identity_never_stores_bad_path() {
        let text = r#"[{
            "session.label": "jenkins",
            "session.protocol": "SSH",
            "session.target": "root@47.239.178.0",
            "session.uuid": "j1",
            "ssh.identityFilePath.windows": "D:\\missing\\old-key.pem"
        }]"#;
        let preview = preview_windterm(text, None).unwrap();
        let c = &preview.candidates[0];
        assert!(c.warnings.iter().any(|w| w.contains("not found")));
        let p = c.profile.as_ref().unwrap();
        if let ProfileKind::Ssh { ssh } = &p.spec {
            match &ssh.auth {
                AuthMethod::PublicKey { key_path, .. } => {
                    assert!(key_path.is_file(), "import must not keep missing key path");
                }
                AuthMethod::Agent => {}
                other => panic!("unexpected auth {other:?}"),
            }
        } else {
            panic!("expected ssh");
        }
    }

    #[test]
    fn without_identity_uses_default_key_or_agent() {
        let text = r#"[{
            "session.label": "plain",
            "session.protocol": "SSH",
            "session.target": "root@10.0.0.1",
            "session.uuid": "w2"
        }]"#;
        let preview = preview_windterm(text, None).unwrap();
        let p = preview.candidates[0].profile.as_ref().unwrap();
        if let ProfileKind::Ssh { ssh } = &p.spec {
            if first_existing_ssh_key().is_some() {
                assert!(matches!(ssh.auth, AuthMethod::PublicKey { .. }));
            } else {
                assert!(matches!(ssh.auth, AuthMethod::Agent));
            }
        } else {
            panic!("expected ssh");
        }
    }

    #[test]
    fn remap_unix_home_identity_maps_existing_file() {
        let Some(home) = crate::ssh::keys::ssh_home_dir() else {
            return;
        };
        let user = std::env::var("USER")
            .or_else(|_| std::env::var("USERNAME"))
            .unwrap_or_else(|_| "user".into());
        let key = home.join(".ssh/id_ed25519");
        if !key.is_file() {
            return;
        }
        let mapped = format!("/home/{user}/.ssh/id_ed25519");
        assert_eq!(
            crate::ssh::keys::remap_unix_home_identity(&mapped),
            Some(key)
        );
    }

    #[test]
    fn resolve_existing_identity_file_skips_missing_paths() {
        assert!(resolve_existing_identity_file("/nonexistent/windterm/key.pem").is_none());
    }
}
