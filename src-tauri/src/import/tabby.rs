use std::path::{Path, PathBuf};

use serde_json::Value;
use uuid::Uuid;

use crate::profile::{Profile, ProfileKind, RemoteDesktopSpec};
use crate::ssh::{AuthMethod, SshProfile};

use super::common::{
    build_profile, error_candidate, home_dir, read_text_file, ready_candidate, strip_utf8_bom,
};
use super::types::{ImportCandidate, ImportDetectPath, ImportDetectResult, ImportPreviewResult};

const SOURCE: &str = "tabby";
const IMPORT_TAG: &str = "import:tabby";

pub fn detect_tabby_paths() -> ImportDetectResult {
    let mut paths = Vec::new();
    if let Some(home) = home_dir() {
        let cfg = home.join(".config").join("tabby").join("config.yaml");
        push_file_path(&mut paths, &cfg, "Tabby config.yaml");
    }
    if let Ok(appdata) = std::env::var("APPDATA") {
        let cfg = PathBuf::from(appdata).join("tabby").join("config.yaml");
        push_file_path(&mut paths, &cfg, "Tabby config.yaml");
    }
    ImportDetectResult { paths }
}

fn push_file_path(out: &mut Vec<ImportDetectPath>, path: &Path, label: &str) {
    if !path.is_file() {
        return;
    }
    out.push(ImportDetectPath {
        path: path.display().to_string(),
        label: label.into(),
    });
}

pub fn read_tabby_file(path: &Path) -> Result<String, String> {
    read_text_file(path).map(strip_utf8_bom)
}

pub fn preview_tabby(text: &str, path: Option<&str>) -> Result<ImportPreviewResult, String> {
    let trimmed = text.trim_start();
    let items = if trimmed.starts_with('{') || trimmed.starts_with('[') {
        parse_tabby_json(trimmed)?
    } else {
        parse_tabby_yaml(trimmed)?
    };
    if items.is_empty() {
        return Err("no Tabby profiles found in file".into());
    }
    let candidates = items
        .into_iter()
        .enumerate()
        .filter_map(|(idx, item)| map_tabby_item(item, idx))
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return Err("no importable Tabby profiles found".into());
    }
    Ok(ImportPreviewResult {
        source: SOURCE.into(),
        path: path.map(String::from),
        stats: super::types::preview_stats(&candidates),
        candidates,
    })
}

#[derive(Debug, Clone)]
enum TabbyItem {
    AeroTab(Profile),
    Tabby {
        name: String,
        kind: String,
        group: Option<String>,
        options: Value,
    },
}

fn parse_tabby_json(text: &str) -> Result<Vec<TabbyItem>, String> {
    let value: Value =
        serde_json::from_str(text).map_err(|e| format!("invalid Tabby JSON: {e}"))?;
    extract_items(value)
}

fn parse_tabby_yaml(text: &str) -> Result<Vec<TabbyItem>, String> {
    let value: Value =
        serde_yaml::from_str(text).map_err(|e| format!("invalid Tabby YAML: {e}"))?;
    extract_items(value)
}

fn extract_items(value: Value) -> Result<Vec<TabbyItem>, String> {
    match value {
        Value::Array(items) => items.into_iter().map(item_from_value).collect(),
        Value::Object(map) => {
            if let Some(profiles) = map.get("profiles").cloned() {
                return match profiles {
                    Value::Array(items) => items.into_iter().map(item_from_value).collect(),
                    other => item_from_value(other).map(|i| vec![i]),
                };
            }
            item_from_value(Value::Object(map)).map(|i| vec![i])
        }
        other => item_from_value(other).map(|i| vec![i]),
    }
}

fn item_from_value(value: Value) -> Result<TabbyItem, String> {
    if let Ok(profile) = serde_json::from_value::<Profile>(value.clone()) {
        return Ok(TabbyItem::AeroTab(profile));
    }
    let map = value
        .as_object()
        .ok_or_else(|| "Tabby profile entry must be an object".to_string())?;
    let kind = map
        .get("kind")
        .or_else(|| map.get("type"))
        .and_then(|v| v.as_str())
        .unwrap_or("ssh")
        .to_ascii_lowercase();
    let name = map
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("Unnamed")
        .to_string();
    let group = map
        .get("group")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let options = map
        .get("options")
        .or_else(|| map.get("ssh"))
        .cloned()
        .unwrap_or(Value::Object(Default::default()));
    Ok(TabbyItem::Tabby {
        name,
        kind,
        group,
        options,
    })
}

fn map_tabby_item(item: TabbyItem, idx: usize) -> Option<ImportCandidate> {
    match item {
        TabbyItem::AeroTab(profile) => Some(map_aerotab_profile(profile, idx)),
        TabbyItem::Tabby {
            name,
            kind,
            group,
            options,
        } => Some(map_tabby_native(name, kind, group, options, idx)),
    }
}

fn map_aerotab_profile(mut profile: Profile, idx: usize) -> ImportCandidate {
    let source_id = format!("tabby:aerotab:{idx}:{}", profile.name);
    let mut warnings = Vec::new();
    profile.id = Uuid::new_v4();
    if !profile.tags.iter().any(|t| t == IMPORT_TAG) {
        profile.tags.push(IMPORT_TAG.to_string());
    }
    sanitize_auth(&mut profile, &mut warnings);
    let kind = match &profile.spec {
        ProfileKind::Ssh { .. } => "ssh",
        ProfileKind::Rdp { .. } => "rdp",
        ProfileKind::Vnc { .. } => "vnc",
    };
    ready_candidate(
        SOURCE,
        source_id,
        profile.name.clone(),
        profile.group.clone(),
        profile.tags.clone(),
        profile.note.clone(),
        kind,
        profile,
        warnings,
    )
}

fn map_tabby_native(
    name: String,
    kind: String,
    group: Option<String>,
    options: Value,
    idx: usize,
) -> ImportCandidate {
    let source_id = format!("tabby:{idx}:{name}");
    let tags = vec![IMPORT_TAG.to_string()];
    let mut warnings = vec!["password not imported; using ssh-agent".into()];

    match kind.as_str() {
        "ssh" => {
            let host = json_str(&options, &["host", "hostname"]).unwrap_or_default();
            if host.is_empty() {
                return error_candidate(
                    SOURCE,
                    source_id,
                    name,
                    "missing host in Tabby options".into(),
                    tags,
                );
            }
            let port = json_u16(&options, &["port"]).unwrap_or(22);
            let user = json_str(&options, &["user", "username"]).unwrap_or_else(|| "root".into());
            if json_str(&options, &["user", "username"]).is_none() {
                warnings.push("no user in Tabby options; defaulting to root".into());
            }
            let auth = tabby_auth(&options, &mut warnings);
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
                Some("Imported from Tabby".into()),
                ProfileKind::Ssh { ssh },
            );
            ready_candidate(
                SOURCE,
                source_id,
                name,
                group,
                tags,
                Some("Imported from Tabby".into()),
                "ssh",
                profile,
                warnings,
            )
        }
        "rdp" => {
            let host = json_str(&options, &["host", "hostname"]).unwrap_or_default();
            if host.is_empty() {
                return error_candidate(
                    SOURCE,
                    source_id,
                    name,
                    "missing host in Tabby RDP options".into(),
                    tags,
                );
            }
            let rdp = RemoteDesktopSpec {
                host,
                port: json_u16(&options, &["port"]).unwrap_or(3389),
                ssh_profile_id: None,
                local_bind_port: 0,
            };
            let profile = build_profile(
                name.clone(),
                group.clone(),
                tags.clone(),
                Some("Imported from Tabby".into()),
                ProfileKind::Rdp { rdp },
            );
            ready_candidate(
                SOURCE,
                source_id,
                name,
                group,
                tags,
                Some("Imported from Tabby".into()),
                "rdp",
                profile,
                warnings,
            )
        }
        other => error_candidate(
            SOURCE,
            source_id,
            name,
            format!("unsupported Tabby profile type: {other}"),
            tags,
        ),
    }
}

fn tabby_auth(options: &Value, warnings: &mut Vec<String>) -> AuthMethod {
    if let Some(keys) = options.get("privateKeys").and_then(|v| v.as_array()) {
        if let Some(first) = keys
            .first()
            .and_then(|k| k.as_str())
            .filter(|s| !s.is_empty())
        {
            return AuthMethod::PublicKey {
                key_path: PathBuf::from(first),
                passphrase: None,
            };
        }
    }
    if options.get("password").is_some() {
        warnings.push("Tabby password ignored; using ssh-agent".into());
    }
    AuthMethod::Agent
}

fn sanitize_auth(profile: &mut Profile, warnings: &mut Vec<String>) {
    if let ProfileKind::Ssh { ssh } = &mut profile.spec {
        if matches!(ssh.auth, AuthMethod::Password { .. }) {
            ssh.auth = AuthMethod::Agent;
            warnings.push("password not imported; using ssh-agent".into());
        }
    }
}

fn json_str(value: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(s) = value.get(*key).and_then(|v| v.as_str()) {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
}

fn json_u16(value: &Value, keys: &[&str]) -> Option<u16> {
    for key in keys {
        if let Some(v) = value.get(*key) {
            if let Some(n) = v.as_u64() {
                return u16::try_from(n).ok();
            }
            if let Some(s) = v.as_str() {
                return s.parse().ok();
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABBY_YAML: &str = r#"
profiles:
  - type: ssh
    name: web-01
    group: Production
    options:
      host: 10.0.0.5
      user: alice
      port: 2222
  - type: local
    name: local shell
    options: {}
"#;

    const AEROTAB_JSON: &str = r#"[
      {
        "id": "00000000-0000-4000-8000-000000000001",
        "name": "imported",
        "kind": "ssh",
        "ssh": {
          "host": "1.2.3.4",
          "port": 22,
          "user": "root",
          "auth": "Agent",
          "jump_via": []
        }
      }
    ]"#;

    #[test]
    fn parses_tabby_yaml_profiles() {
        let preview = preview_tabby(TABBY_YAML, None).expect("preview");
        assert_eq!(preview.candidates.len(), 2);
        assert_eq!(preview.candidates[0].name, "web-01");
        assert_eq!(
            preview.candidates[1].status,
            super::super::types::ImportCandidateStatus::Error
        );
    }

    #[test]
    fn parses_aerotab_json_array() {
        let preview = preview_tabby(AEROTAB_JSON, None).expect("preview");
        assert_eq!(preview.candidates.len(), 1);
        assert_eq!(preview.candidates[0].name, "imported");
        assert!(preview.candidates[0]
            .tags
            .contains(&"import:tabby".to_string()));
    }
}
