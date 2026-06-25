use std::collections::HashMap;
use std::path::Path;

use crate::profile::{ProfileKind, RemoteDesktopSpec};
use crate::ssh::{AuthMethod, SshProfile};

use super::common::{
    build_profile, error_candidate, read_text_file, ready_candidate, strip_utf8_bom,
};
use super::types::{ImportCandidate, ImportDetectResult, ImportPreviewResult};

const SOURCE: &str = "csv";
const IMPORT_TAG: &str = "import:csv";

pub fn detect_csv_paths() -> ImportDetectResult {
    ImportDetectResult { paths: vec![] }
}

pub fn read_csv_file(path: &Path) -> Result<String, String> {
    read_text_file(path).map(strip_utf8_bom)
}

pub fn preview_csv(text: &str, path: Option<&str>) -> Result<ImportPreviewResult, String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let rows = parse_csv_rows(text)?;
    if rows.is_empty() {
        return Err("CSV file has no data rows".into());
    }
    let header = &rows[0];
    let delim = detect_delimiter(header);
    let columns = parse_csv_line(header, delim);
    let field_map = map_columns(&columns);
    if !field_map.contains_key("host") {
        return Err("CSV must include a host column".into());
    }

    let mut candidates = Vec::new();
    for (row_idx, raw) in rows.iter().skip(1).enumerate() {
        if raw.trim().is_empty() {
            continue;
        }
        let cells = parse_csv_line(raw, delim);
        candidates.push(map_row(row_idx + 2, &cells, &field_map));
    }
    if candidates.is_empty() {
        return Err("CSV file has no data rows".into());
    }
    Ok(ImportPreviewResult {
        source: SOURCE.into(),
        path: path.map(String::from),
        stats: super::types::preview_stats(&candidates),
        candidates,
    })
}

fn detect_delimiter(header: &str) -> char {
    let semi = header.matches(';').count();
    let comma = header.matches(',').count();
    if semi > comma {
        ';'
    } else {
        ','
    }
}

fn map_columns(headers: &[String]) -> HashMap<&'static str, usize> {
    let mut out = HashMap::new();
    for (idx, raw) in headers.iter().enumerate() {
        let key = raw.trim().to_ascii_lowercase();
        let field = match key.as_str() {
            "name" | "label" | "session" | "title" => Some("name"),
            "host" | "hostname" | "host_name" | "address" | "ip" => Some("host"),
            "port" => Some("port"),
            "user" | "username" | "login" => Some("user"),
            "group" => Some("group"),
            "tags" => Some("tags"),
            "protocol" | "type" | "kind" => Some("protocol"),
            "key_path" | "identityfile" | "identity_file" | "key" | "private_key" => {
                Some("key_path")
            }
            "notes" | "note" | "description" | "comment" => Some("notes"),
            _ => None,
        };
        if let Some(f) = field {
            out.entry(f).or_insert(idx);
        }
    }
    out
}

fn cell(cells: &[String], field_map: &HashMap<&str, usize>, key: &str) -> Option<String> {
    let idx = *field_map.get(key)?;
    cells.get(idx).map(|s| s.trim().to_string())
}

fn map_row(line_no: usize, cells: &[String], field_map: &HashMap<&str, usize>) -> ImportCandidate {
    let source_id = format!("csv:{line_no}");
    let host = cell(cells, field_map, "host").unwrap_or_default();
    let name = cell(cells, field_map, "name")
        .filter(|s| !s.is_empty())
        .or_else(|| {
            let user = cell(cells, field_map, "user");
            if host.is_empty() {
                None
            } else if let Some(u) = user.filter(|u| !u.is_empty()) {
                Some(format!("{u}@{host}"))
            } else {
                Some(host.clone())
            }
        })
        .unwrap_or_else(|| format!("Row {line_no}"));
    let mut tags = vec![IMPORT_TAG.to_string()];
    if let Some(extra) = cell(cells, field_map, "tags") {
        for t in extra.split(&[',', ';'][..]) {
            let t = t.trim();
            if !t.is_empty() {
                tags.push(t.to_string());
            }
        }
    }
    tags.sort();
    tags.dedup();

    if host.is_empty() {
        return error_candidate(SOURCE, source_id, name, "missing host".into(), tags);
    }

    let protocol = cell(cells, field_map, "protocol")
        .unwrap_or_else(|| "ssh".into())
        .to_ascii_lowercase();
    let port = cell(cells, field_map, "port").and_then(|p| p.parse::<u16>().ok());
    let user = cell(cells, field_map, "user");
    let group = cell(cells, field_map, "group");
    let note = cell(cells, field_map, "notes");
    let key_path = cell(cells, field_map, "key_path");

    let mut warnings = Vec::new();
    if key_path.is_none() && protocol == "ssh" {
        warnings.push("no key_path; using ssh-agent".into());
    }

    let (kind, profile): (String, crate::profile::Profile) = match protocol.as_str() {
        "ssh" | "sftp" | "" => {
            let auth = key_path
                .as_ref()
                .filter(|p| !p.is_empty())
                .map(|p| AuthMethod::PublicKey {
                    key_path: p.into(),
                    passphrase: None,
                })
                .unwrap_or(AuthMethod::Agent);
            let ssh = SshProfile {
                host: host.clone(),
                port: port.unwrap_or(22),
                user: user.unwrap_or_else(|| "root".into()),
                auth,
                jump_via: vec![],
            };
            (
                "ssh".to_string(),
                build_profile(
                    name.clone(),
                    group.clone(),
                    tags.clone(),
                    note.clone(),
                    ProfileKind::Ssh { ssh },
                ),
            )
        }
        "rdp" => {
            let rdp = RemoteDesktopSpec {
                host: host.clone(),
                port: port.unwrap_or(3389),
                ssh_profile_id: None,
                local_bind_port: 0,
            };
            (
                "rdp".to_string(),
                build_profile(
                    name.clone(),
                    group.clone(),
                    tags.clone(),
                    note.clone(),
                    ProfileKind::Rdp { rdp },
                ),
            )
        }
        "vnc" => {
            let spec = RemoteDesktopSpec {
                host: host.clone(),
                port: port.unwrap_or(5900),
                ssh_profile_id: None,
                local_bind_port: 0,
            };
            (
                "vnc".to_string(),
                build_profile(
                    name.clone(),
                    group.clone(),
                    tags.clone(),
                    note.clone(),
                    ProfileKind::Vnc { spec },
                ),
            )
        }
        other => {
            return error_candidate(
                SOURCE,
                source_id,
                name,
                format!("unsupported protocol: {other}"),
                tags,
            );
        }
    };

    ready_candidate(
        SOURCE,
        source_id,
        name,
        group,
        tags,
        note,
        kind.as_str(),
        profile,
        warnings,
    )
}

fn parse_csv_rows(text: &str) -> Result<Vec<String>, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("CSV file is empty".into());
    }
    Ok(trimmed.lines().map(str::to_string).collect())
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

    const SAMPLE: &str = "\
name,host,port,user,group,tags,protocol,key_path,notes
web-01,10.0.0.5,2222,alice,Production,\"ops,prod\",ssh,/home/alice/.ssh/id_ed25519,Primary
desk,192.168.1.20,,,,,rdp,,
bad-row,,,,,,telnet,,
";

    #[test]
    fn parses_ssh_rdp_and_error_rows() {
        let preview = preview_csv(SAMPLE, None).expect("preview");
        assert_eq!(preview.candidates.len(), 3);
        let web = &preview.candidates[0];
        assert_eq!(web.name, "web-01");
        assert_eq!(web.kind, "ssh");
        assert!(web.tags.contains(&"prod".to_string()));
        let desk = &preview.candidates[1];
        assert_eq!(desk.kind, "rdp");
        assert_eq!(
            preview.candidates[2].status,
            super::super::types::ImportCandidateStatus::Error
        );
    }

    #[test]
    fn accepts_semicolon_delimiter() {
        let csv = "name;host;port\nsrv;1.2.3.4;22\n";
        let preview = preview_csv(csv, None).expect("preview");
        assert_eq!(preview.candidates.len(), 1);
        assert_eq!(preview.candidates[0].name, "srv");
    }

    #[test]
    fn strips_bom() {
        let csv = "\u{feff}name,host\nfoo,1.1.1.1\n";
        let preview = preview_csv(csv, None).expect("preview");
        assert_eq!(preview.candidates[0].name, "foo");
    }
}
