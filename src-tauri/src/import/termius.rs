use std::collections::HashMap;
use std::path::Path;

use serde_json::Value;

use crate::profile::{ProfileKind, RemoteDesktopSpec};
use crate::ssh::{AuthMethod, SshProfile};

use super::common::{
    build_profile, error_candidate, read_text_file, ready_candidate, strip_utf8_bom,
};
use super::types::{ImportCandidate, ImportDetectResult, ImportPreviewResult};

const SOURCE: &str = "termius";
const IMPORT_TAG: &str = "import:termius";

pub fn detect_termius_paths() -> ImportDetectResult {
    ImportDetectResult { paths: vec![] }
}

pub fn read_termius_file(path: &Path) -> Result<String, String> {
    read_text_file(path).map(strip_utf8_bom)
}

pub fn preview_termius(text: &str, path: Option<&str>) -> Result<ImportPreviewResult, String> {
    let trimmed = text.trim_start();
    let candidates = if trimmed.starts_with('{') || trimmed.starts_with('[') {
        preview_termius_json(trimmed)?
    } else {
        preview_termius_csv(trimmed)?
    };
    if candidates.is_empty() {
        return Err("no importable Termius hosts found".into());
    }
    Ok(ImportPreviewResult {
        source: SOURCE.into(),
        path: path.map(String::from),
        stats: super::types::preview_stats(&candidates),
        candidates,
    })
}

fn preview_termius_csv(text: &str) -> Result<Vec<ImportCandidate>, String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let rows: Vec<String> = text.lines().map(str::to_string).collect();
    if rows.is_empty() {
        return Err("Termius CSV is empty".into());
    }
    let header = &rows[0];
    let delim = if header.matches(';').count() > header.matches(',').count() {
        ';'
    } else {
        ','
    };
    let columns = parse_csv_line(header, delim);
    let field_map = map_termius_columns(&columns);
    if !field_map.contains_key("host") {
        return Err("Termius CSV must include Hostname/IP column".into());
    }

    let mut out = Vec::new();
    for (row_idx, raw) in rows.iter().skip(1).enumerate() {
        if raw.trim().is_empty() {
            continue;
        }
        let cells = parse_csv_line(raw, delim);
        out.push(map_csv_row(row_idx + 2, &cells, &field_map));
    }
    Ok(out)
}

fn preview_termius_json(text: &str) -> Result<Vec<ImportCandidate>, String> {
    let value: Value =
        serde_json::from_str(text).map_err(|e| format!("invalid Termius JSON: {e}"))?;
    let items = extract_host_objects(value);
    if items.is_empty() {
        return Err("no host entries in Termius JSON".into());
    }
    Ok(items
        .into_iter()
        .enumerate()
        .map(|(idx, item)| map_json_host(&item, idx))
        .collect())
}

fn extract_host_objects(value: Value) -> Vec<Value> {
    match value {
        Value::Array(items) => items,
        Value::Object(map) => {
            for key in ["hosts", "hostDefinitions", "connections", "items"] {
                if let Some(Value::Array(items)) = map.get(key) {
                    return items.clone();
                }
            }
            if map.contains_key("address")
                || map.contains_key("hostname")
                || map.contains_key("label")
            {
                return vec![Value::Object(map)];
            }
            Vec::new()
        }
        _ => Vec::new(),
    }
}

fn map_termius_columns(headers: &[String]) -> HashMap<&'static str, usize> {
    let mut out = HashMap::new();
    for (idx, raw) in headers.iter().enumerate() {
        let key = raw.trim().to_ascii_lowercase();
        let field = match key.as_str() {
            "groups" | "group" | "folder" => Some("group"),
            "label" | "name" | "title" => Some("name"),
            "tags" => Some("tags"),
            "hostname/ip" | "hostname" | "host" | "address" | "ip" => Some("host"),
            "protocol" => Some("protocol"),
            "port" => Some("port"),
            "username" | "user" => Some("user"),
            "password" => Some("password"),
            _ => None,
        };
        if let Some(f) = field {
            out.entry(f).or_insert(idx);
        }
    }
    out
}

fn map_csv_row(
    line_no: usize,
    cells: &[String],
    field_map: &HashMap<&str, usize>,
) -> ImportCandidate {
    let host = cell(cells, field_map, "host").unwrap_or_default();
    let name = cell(cells, field_map, "name")
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            if host.is_empty() {
                format!("Row {line_no}")
            } else {
                host.clone()
            }
        });
    let source_id = format!("termius:csv:{line_no}:{name}");
    let group = cell(cells, field_map, "group");
    let protocol = cell(cells, field_map, "protocol").unwrap_or_else(|| "ssh".into());
    let port = cell(cells, field_map, "port");
    let user = cell(cells, field_map, "user");
    let password_present = cell(cells, field_map, "password").is_some_and(|p| !p.is_empty());
    let tags = merge_tags(cell(cells, field_map, "tags"));
    map_host(
        source_id,
        name,
        group,
        tags,
        host,
        &protocol,
        port.as_deref(),
        user.as_deref(),
        password_present,
        None,
    )
}

fn map_json_host(obj: &Value, idx: usize) -> ImportCandidate {
    let name = json_str(obj, &["label", "name", "title"]).unwrap_or_else(|| format!("Host {idx}"));
    let source_id = format!(
        "termius:json:{idx}:{}",
        json_str(obj, &["id", "hostId", "host_id"]).unwrap_or_else(|| name.clone())
    );
    let group = json_str(obj, &["groups", "group", "folder"]);
    let tags = merge_tags(json_str(obj, &["tags"]));
    let host = json_str(obj, &["address", "hostname", "host", "ip"]).unwrap_or_default();
    let protocol =
        json_str(obj, &["protocol", "connection_type", "type"]).unwrap_or_else(|| "ssh".into());
    let port = json_str(obj, &["port"]).or_else(|| ssh_nested_str(obj, &["port"]));
    let user =
        json_str(obj, &["username", "user"]).or_else(|| ssh_nested_str(obj, &["username", "user"]));
    let password_present = json_str(obj, &["password"]).is_some()
        || obj.get("password").map(|v| !v.is_null()).unwrap_or(false);
    map_host(
        source_id,
        name,
        group,
        tags,
        host,
        &protocol,
        port.as_deref(),
        user.as_deref(),
        password_present,
        json_str(obj, &["notes", "note", "description"]),
    )
}

#[allow(clippy::too_many_arguments)]
fn map_host(
    source_id: String,
    name: String,
    group: Option<String>,
    mut tags: Vec<String>,
    host: String,
    protocol: &str,
    port: Option<&str>,
    user: Option<&str>,
    password_present: bool,
    note: Option<String>,
) -> ImportCandidate {
    if !tags.iter().any(|t| t == IMPORT_TAG) {
        tags.insert(0, IMPORT_TAG.to_string());
    }
    tags.sort();
    tags.dedup();

    if host.trim().is_empty() {
        return error_candidate(SOURCE, source_id, name, "missing host address".into(), tags);
    }

    let protocol_lc = normalize_protocol(protocol);
    let mut warnings = Vec::new();
    if password_present {
        warnings.push("password not imported; using ssh-agent".into());
    }

    match protocol_lc.as_str() {
        "ssh" | "sftp" => {
            let port = port.and_then(|p| p.parse::<u16>().ok()).unwrap_or(22);
            if user.is_none() {
                warnings.push("no username; defaulting to root".into());
            }
            let user = user.unwrap_or("root").to_string();
            let group = normalize_group(group);
            let ssh = SshProfile {
                host: host.clone(),
                port,
                user,
                auth: AuthMethod::Agent,
                jump_via: vec![],
            };
            let profile = build_profile(
                name.clone(),
                group.clone(),
                tags.clone(),
                note.or_else(|| Some("Imported from Termius".into())),
                ProfileKind::Ssh { ssh },
            );
            ready_candidate(
                SOURCE,
                source_id,
                name,
                group,
                tags,
                Some("Imported from Termius".into()),
                "ssh",
                profile,
                warnings,
            )
        }
        "rdp" => {
            let port = port.and_then(|p| p.parse::<u16>().ok()).unwrap_or(3389);
            let group = normalize_group(group);
            let rdp = RemoteDesktopSpec {
                host: host.clone(),
                port,
                ssh_profile_id: None,
                local_bind_port: 0,
            };
            let profile = build_profile(
                name.clone(),
                group.clone(),
                tags.clone(),
                note.or_else(|| Some("Imported from Termius".into())),
                ProfileKind::Rdp { rdp },
            );
            ready_candidate(
                SOURCE,
                source_id,
                name,
                group,
                tags,
                Some("Imported from Termius".into()),
                "rdp",
                profile,
                warnings,
            )
        }
        other => error_candidate(
            SOURCE,
            source_id,
            name,
            format!("unsupported Termius protocol: {other}"),
            tags,
        ),
    }
}

fn normalize_protocol(raw: &str) -> String {
    let p = raw.trim().to_ascii_lowercase();
    if p.is_empty() || p.contains("ssh") || p == "sftp" {
        return "ssh".into();
    }
    if p.contains("telnet") {
        return "telnet".into();
    }
    if p.contains("mosh") {
        return "mosh".into();
    }
    if p.contains("rdp") {
        return "rdp".into();
    }
    if p.contains("vnc") {
        return "vnc".into();
    }
    p
}

fn normalize_group(group: Option<String>) -> Option<String> {
    group
        .map(|g| g.trim().replace('\\', "/"))
        .filter(|g| !g.is_empty())
}

fn merge_tags(extra: Option<String>) -> Vec<String> {
    let mut tags = vec![IMPORT_TAG.to_string()];
    if let Some(raw) = extra {
        for t in raw.split(&[',', ';'][..]) {
            let t = t.trim();
            if !t.is_empty() {
                tags.push(t.to_string());
            }
        }
    }
    tags
}

fn cell(cells: &[String], field_map: &HashMap<&str, usize>, key: &str) -> Option<String> {
    let idx = *field_map.get(key)?;
    cells.get(idx).map(|s| s.trim().to_string())
}

fn json_str(value: &Value, keys: &[&str]) -> Option<String> {
    let map = value.as_object()?;
    for key in keys {
        if let Some(v) = map.get(*key) {
            if let Some(s) = v.as_str() {
                if !s.is_empty() {
                    return Some(s.to_string());
                }
            }
        }
    }
    None
}

fn ssh_nested_str(value: &Value, keys: &[&str]) -> Option<String> {
    let ssh = value.get("ssh").or_else(|| value.get("sshConfig"))?;
    json_str(ssh, keys)
}

fn parse_csv_line(line: &str, delim: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '"' if !in_quotes => in_quotes = true,
            '"' if in_quotes => {
                if matches!(chars.peek(), Some('"')) {
                    chars.next();
                    cur.push('"');
                } else {
                    in_quotes = false;
                }
            }
            c if c == delim && !in_quotes => {
                out.push(cur.trim().to_string());
                cur.clear();
            }
            c => cur.push(c),
        }
    }
    out.push(cur.trim().to_string());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const CSV: &str = "\
Groups,Label,Tags,Hostname/IP,Protocol,Port,Username,Password
Production/web,web-01,\"ops,prod\",10.0.0.5,SSH,2222,alice,
AWS,desk,,192.168.1.20,RDP,3389,admin,secret
bad,,,,,Telnet,23,,
";

    const JSON: &str = r#"[
      {
        "label": "api-01",
        "address": "10.0.0.8",
        "protocol": "SSH",
        "port": 22,
        "username": "deploy",
        "groups": "Staging"
      }
    ]"#;

    #[test]
    fn parses_termius_csv() {
        let preview = preview_termius(CSV, None).expect("preview");
        assert_eq!(preview.candidates.len(), 3);
        let web = &preview.candidates[0];
        assert_eq!(web.name, "web-01");
        assert_eq!(web.group.as_deref(), Some("Production/web"));
        assert!(web.tags.contains(&"prod".to_string()));
        assert_eq!(
            preview.candidates[2].status,
            super::super::types::ImportCandidateStatus::Error
        );
    }

    #[test]
    fn parses_termius_json() {
        let preview = preview_termius(JSON, None).expect("preview");
        assert_eq!(preview.candidates.len(), 1);
        assert_eq!(preview.candidates[0].name, "api-01");
        assert_eq!(preview.candidates[0].group.as_deref(), Some("Staging"));
    }
}
